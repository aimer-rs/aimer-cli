use super::*;

struct RuntimeResources {
    children: Vec<Child>,
    host_build: Option<Child>,
    guest_build: Option<Child>,
    startup_cancelled: bool,
    announcements: Option<Receiver<String>>,
    watcher: Option<RecommendedWatcher>,
    changes: Option<Receiver<notify::Result<notify::Event>>>,
    client: Option<ReloadClient>,
    android_route: Option<RouteReservation<SystemCommandExecutor>>,
    android_session: Option<OwnedCommandGuard<SystemCommandExecutor>>,
    exact_watch_roots: Option<Vec<PathBuf>>,
}

impl RuntimeResources {
    const fn new() -> Self {
        Self {
            children: Vec::new(),
            host_build: None,
            guest_build: None,
            startup_cancelled: false,
            announcements: None,
            watcher: None,
            changes: None,
            client: None,
            android_route: None,
            android_session: None,
            exact_watch_roots: None,
        }
    }
}

/// Production runtime backed by `std::process`, TCP, and `notify`.
///
/// Child handles, Android route guards, the authenticated client, and the
/// filesystem watcher live in one mutex so cleanup can atomically take and
/// release everything created by this run.
pub struct SystemProcessRuntime {
    resources: Mutex<RuntimeResources>,
    executor: Arc<SystemCommandExecutor>,
}

impl SystemProcessRuntime {
    /// Creates an empty production runtime.
    #[inline]
    pub fn new() -> Self {
        Self {
            resources: Mutex::new(RuntimeResources::new()),
            executor: Arc::new(SystemCommandExecutor),
        }
    }

    fn run_build(&self, root: &Path, build: &BuildCommand, guest: bool) -> Result<(), String> {
        let child = Command::new(build.program())
            .args(build.arguments())
            .current_dir(root)
            .spawn()
            .map_err(|error| format!("failed to start {}: {error}", build.program()))?;

        {
            let mut resources = self.resources.lock().map_err(lock_error)?;
            if resources.startup_cancelled {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err("hot-reload startup was cancelled".to_owned());
            }
            let slot = if guest {
                &mut resources.guest_build
            } else {
                &mut resources.host_build
            };
            if slot.is_some() {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(if guest {
                    "an initial guest compiler is already running".to_owned()
                } else {
                    "a native host compiler is already running".to_owned()
                });
            }
            *slot = Some(child);
        }

        loop {
            let poll = {
                let mut resources = self.resources.lock().map_err(lock_error)?;
                if resources.startup_cancelled {
                    let slot = if guest {
                        &mut resources.guest_build
                    } else {
                        &mut resources.host_build
                    };
                    terminate_child(slot);
                    return Err("hot-reload startup was cancelled".to_owned());
                }
                let slot = if guest {
                    &mut resources.guest_build
                } else {
                    &mut resources.host_build
                };
                slot.as_mut()
                    .ok_or_else(|| "hot-reload compiler process disappeared".to_owned())?
                    .try_wait()
            };
            match poll {
                Ok(Some(status)) => {
                    let mut resources = self.resources.lock().map_err(lock_error)?;
                    let slot = if guest {
                        &mut resources.guest_build
                    } else {
                        &mut resources.host_build
                    };
                    slot.take();
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(format!("{} exited with {status}", build.program()))
                    };
                }
                Ok(None) => thread::sleep(Duration::from_millis(20)),
                Err(error) => {
                    let mut resources = self.resources.lock().map_err(lock_error)?;
                    let slot = if guest {
                        &mut resources.guest_build
                    } else {
                        &mut resources.host_build
                    };
                    slot.take();
                    return Err(format!("failed to wait for {}: {error}", build.program()));
                }
            }
        }
    }

    fn execute_step(root: &Path, step: &LaunchStep) -> Result<(), String> {
        let mut command = Self::step_command(root, step);
        command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        if step.stdin().is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to start {}: {error}", step.command().program()))?;
        if let Some(payload) = step.stdin() {
            child
                .stdin
                .take()
                .ok_or_else(|| "private launch input pipe was unavailable".to_owned())?
                .write_all(payload.as_bytes())
                .map_err(|error| format!("failed to provide private launch input: {error}"))?;
        }
        let status = child
            .wait()
            .map_err(|error| format!("failed to wait for {}: {error}", step.command().program()))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{} exited with {status}", step.command().program()))
        }
    }

    fn spawn_step(root: &Path, step: &LaunchStep) -> Result<(Child, Receiver<String>), String> {
        let mut command = Self::step_command(root, step);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        if step.stdin().is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to launch {}: {error}", step.command().program()))?;
        if let Some(payload) = step.stdin() {
            child
                .stdin
                .take()
                .ok_or_else(|| "private launch input pipe was unavailable".to_owned())?
                .write_all(payload.as_bytes())
                .map_err(|error| format!("failed to provide private launch input: {error}"))?;
        }
        let (sender, receiver) = mpsc::channel();
        if let Some(stdout) = child.stdout.take() {
            stream_lines(stdout, sender.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            stream_lines(stderr, sender);
        }
        Ok((child, receiver))
    }

    fn step_command(root: &Path, step: &LaunchStep) -> Command {
        let mut command = Command::new(step.command().program());
        command.args(step.command().arguments()).current_dir(root);
        for (name, value) in step.environment() {
            command.env(name, value.as_str());
        }
        command
    }

    fn launch_configuration(
        &self,
        root: &Path,
        device: &SystemDevice,
        launch: LaunchConfiguration,
    ) -> Result<(), String> {
        let app_index = launch.steps().len() - 1;
        for step in &launch.steps()[..app_index] {
            Self::execute_step(root, step)?;
        }
        if device.family == TargetFamily::Android {
            Self::execute_step(root, launch.app())?;
            let logcat = LaunchStep::new(crate::route::CommandSpec::new(
                "adb",
                vec!["-s".into(), device.id.clone(), "logcat".into()],
            ));
            let (child, announcements) = Self::spawn_step(root, &logcat)?;
            let mut resources = self.resources.lock().map_err(lock_error)?;
            resources.children.push(child);
            resources.announcements = Some(announcements);
        } else {
            let (child, announcements) = Self::spawn_step(root, launch.app())?;
            let mut resources = self.resources.lock().map_err(lock_error)?;
            resources.children.push(child);
            resources.announcements = Some(announcements);
        }
        Ok(())
    }
}

impl Default for SystemProcessRuntime {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl SystemRuntime for SystemProcessRuntime {
    fn build_host(
        &self,
        project_root: &Path,
        _: &Path,
        package: &str,
        _: &SystemDevice,
    ) -> Result<(), String> {
        let plan = HotReloadBuildPlan::new(
            project_root.join("Cargo.toml"),
            "aimer_generated_guest",
            package,
            project_root.join("target/aimer-hot-reload/guest"),
        );
        self.run_build(project_root, &plan.host_command(), false)
    }

    fn build_guest(
        &self,
        project_root: &Path,
        workspace_root: &Path,
        package: &str,
        _: u64,
    ) -> Result<Vec<u8>, String> {
        let manifest = AimerManifest::load_from(project_root)
            .map_err(|error| format!("failed to load hot-reload metadata: {error}"))?;
        let session_root = project_root.join("target/aimer-hot-reload");
        let target_root = session_root.join("guest");
        let (generated, host_package) = match GuestGenerationMode::select(manifest.as_ref()) {
            GuestGenerationMode::Automatic => (
                prepare_automatic_guest(project_root, workspace_root, &session_root)
                    .map_err(|error| error.to_string())?,
                package,
            ),
            GuestGenerationMode::Manual => {
                let guest = manifest
                    .as_ref()
                    .and_then(AimerManifest::hot_reload_guest)
                    .expect("manual mode has explicit metadata");
                (
                    generate_guest_package(
                        project_root,
                        &workspace_root.join("crates/aimer_wasm_guest"),
                        &session_root.join("generated"),
                        &GuestPackageSpec::new(guest.package(), guest.program(), guest.limits()),
                    )
                    .map_err(|error| error.to_string())?,
                    guest.package(),
                )
            }
        };
        let plan = HotReloadBuildPlan::new(
            project_root.join("Cargo.toml"),
            generated.package(),
            host_package,
            target_root.clone(),
        )
        .guest_manifest(generated.manifest().to_owned())
        .guest_source_remap(
            generated.application_root().to_owned(),
            project_root.to_owned(),
        );
        self.run_build(project_root, &plan.guest_command(), true)?;
        let artifact = target_root
            .join("wasm32-unknown-unknown/debug")
            .join(format!("{}.wasm", generated.package().replace('-', "_")));
        let module = load_guest_artifact(
            &artifact,
            &target_root,
            DevelopmentHostConfig::cli_safe_profile().module_bytes_limit(),
        )
        .map_err(|error| error.to_string())?;
        if let Some(portable_source_root) = generated.portable_source_root() {
            let watch_set = WatchSet::automatic(
                project_root.to_owned(),
                portable_source_root.to_owned(),
                local_path_dependencies(project_root),
                vec![
                    workspace_root.join("target"),
                    project_root.join("target"),
                    project_root.join("builds"),
                ],
            );
            self.resources.lock().map_err(lock_error)?.exact_watch_roots =
                Some(watch_set.watch_roots().to_vec());
        }
        Ok(module)
    }

    fn cancel_startup(&self) {
        let Ok(mut resources) = self.resources.lock() else {
            return;
        };
        resources.startup_cancelled = true;
        terminate_child(&mut resources.host_build);
        terminate_child(&mut resources.guest_build);
    }

    fn assemble(
        &self,
        project_root: &Path,
        _: &Path,
        package: &str,
        device: &SystemDevice,
    ) -> Result<AssembledHost, String> {
        match device.family {
            TargetFamily::Windows | TargetFamily::Linux => Ok(AssembledHost::DesktopExecutable(
                project_root
                    .join("target/debug")
                    .join(format!("{package}{}", std::env::consts::EXE_SUFFIX)),
            )),
            TargetFamily::Macos => Ok(AssembledHost::DesktopExecutable(
                project_root
                    .join("builds/macos/build/Debug")
                    .join(format!("{package}.app/Contents/MacOS/{package}")),
            )),
            TargetFamily::IosSimulator | TargetFamily::IosDevice => {
                Ok(AssembledHost::MobileApplication {
                    bundle_id: mobile_application_id(project_root, package),
                })
            }
            TargetFamily::Android => Ok(AssembledHost::MobileApplication {
                bundle_id: android_application_id(project_root, package),
            }),
            TargetFamily::Web => Err("web cannot assemble a native hot-reload host".to_owned()),
        }
    }

    fn prepare_route(&self, device: &SystemDevice, listener_port: u16) -> Result<(), String> {
        if device.family == TargetFamily::Android {
            let adapter = AndroidRouteAdapter::new(Arc::clone(&self.executor));
            let reservation = adapter
                .prepare(&device.id, listener_port)
                .map_err(|error| error.to_string())?;
            self.resources.lock().map_err(lock_error)?.android_route = Some(reservation);
        }
        Ok(())
    }

    fn launch(
        &self,
        device: &SystemDevice,
        package: &str,
        host: &AssembledHost,
        session: &DevelopmentSession,
    ) -> Result<(), String> {
        let launch = match (device.family, host) {
            (
                TargetFamily::Macos | TargetFamily::Windows | TargetFamily::Linux,
                AssembledHost::DesktopExecutable(binary),
            ) => DesktopRouteAdapter::new(device.family, binary).launch(session),
            (TargetFamily::IosSimulator, AssembledHost::MobileApplication { bundle_id }) => {
                SimulatorRouteAdapter::new(&device.id, bundle_id).launch(session)
            }
            (TargetFamily::IosDevice, AssembledHost::MobileApplication { bundle_id }) => {
                IosDeviceRouteAdapter::new(&device.id, bundle_id).launch(session)
            }
            (TargetFamily::Android, AssembledHost::MobileApplication { bundle_id }) => {
                let adapter = AndroidRouteAdapter::new(Arc::clone(&self.executor));
                self.resources.lock().map_err(lock_error)?.android_session =
                    Some(adapter.session_file_guard(&device.id, bundle_id));
                adapter.launch(&device.id, bundle_id, session)
            }
            _ => return Err(format!("assembled host does not match {}", device.family)),
        };
        self.launch_configuration(Path::new("."), device, launch)
            .map_err(|error| format!("failed to launch {package} on {}: {error}", device.name))
    }

    fn connect(&self, device: &SystemDevice, session: &DevelopmentSession) -> Result<(), String> {
        let announcements = self
            .resources
            .lock()
            .map_err(lock_error)?
            .announcements
            .take()
            .ok_or_else(|| "launch output is unavailable for listener discovery".to_owned())?;
        let readiness = await_listener_readiness(
            &announcements,
            *session.credentials().session_id(),
            PROTOCOL_VERSION,
            device.family.name(),
            READINESS_TIMEOUT,
        )
        .map_err(|error| error.to_string())?;
        let address = match device.family {
            TargetFamily::Macos | TargetFamily::Windows | TargetFamily::Linux => {
                DesktopRouteAdapter::new(device.family, "unused")
                    .endpoint(&readiness)
                    .map_err(|error| error.to_string())?
                    .address()
            }
            TargetFamily::IosSimulator => SimulatorRouteAdapter::new(&device.id, "unused")
                .endpoint(&readiness)
                .map_err(|error| error.to_string())?
                .address(),
            TargetFamily::Android => {
                let resources = self.resources.lock().map_err(lock_error)?;
                let route = resources
                    .android_route
                    .as_ref()
                    .ok_or_else(|| "Android route was not prepared".to_owned())?;
                AndroidRouteAdapter::new(Arc::clone(&self.executor))
                    .endpoint(route)
                    .map_err(|error| error.to_string())?
                    .address()
            }
            TargetFamily::IosDevice => {
                let service =
                    IosDeviceRouteAdapter::service_name(*session.credentials().session_id());
                let discovery =
                    LaunchStep::new(IosDeviceRouteAdapter::resolution_command(&service));
                let (child, resolutions) = Self::spawn_step(Path::new("."), &discovery)?;
                self.resources
                    .lock()
                    .map_err(lock_error)?
                    .children
                    .push(child);
                IosDeviceRouteAdapter::resolve(
                    &resolutions,
                    &readiness,
                    &service,
                    READINESS_TIMEOUT,
                )
                .map_err(|error| error.to_string())?
                .address()
            }
            TargetFamily::Web => return Err("web has no native reload listener".to_owned()),
        };
        let limits = client_protocol_limits(session.host_config());
        self.resources.lock().map_err(lock_error)?.client = Some(ReloadClient::new(
            address,
            session.credentials().clone(),
            limits,
        ));
        Ok(())
    }

    fn push_reload(
        &self,
        request_id: u64,
        metadata: ModuleMetadata,
        module: &[u8],
    ) -> Result<ReloadResult, String> {
        let resources = self.resources.lock().map_err(lock_error)?;
        resources
            .client
            .as_ref()
            .ok_or_else(|| "reload client is not connected".to_owned())?
            .push_reload(request_id, metadata, module)
            .map_err(client_error)
    }

    fn recover_result(&self, request_id: u64) -> Result<Option<ReloadResult>, String> {
        let resources = self.resources.lock().map_err(lock_error)?;
        resources
            .client
            .as_ref()
            .ok_or_else(|| "reload client is not connected".to_owned())?
            .query_result(request_id)
            .map_err(client_error)
    }

    fn start_watcher(&self, _: &Path, watch_roots: &[PathBuf]) -> Result<(), String> {
        let (sender, receiver) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        })
        .map_err(|error| format!("failed to create source watcher: {error}"))?;
        let exact_watch_roots = self
            .resources
            .lock()
            .map_err(lock_error)?
            .exact_watch_roots
            .clone();
        let watch_roots = exact_watch_roots.as_deref().unwrap_or(watch_roots);
        for root in watch_roots.iter().filter(|root| root.exists()) {
            watcher
                .watch(root, RecursiveMode::Recursive)
                .map_err(|error| format!("failed to watch {}: {error}", root.display()))?;
        }
        let mut resources = self.resources.lock().map_err(lock_error)?;
        resources.watcher = Some(watcher);
        resources.changes = Some(receiver);
        Ok(())
    }

    fn next_change(&self, timeout: Duration) -> Result<Option<Vec<PathBuf>>, String> {
        let resources = self.resources.lock().map_err(lock_error)?;
        let receiver = resources
            .changes
            .as_ref()
            .ok_or_else(|| "source watcher was not started".to_owned())?;
        match receiver.recv_timeout(timeout) {
            Ok(Ok(event)) => {
                let mut paths = event.paths;
                while let Ok(event) = receiver.recv_timeout(Duration::from_millis(75)) {
                    paths.extend(
                        event
                            .map_err(|error| format!("source watcher failed: {error}"))?
                            .paths,
                    );
                }
                Ok(Some(paths))
            }
            Ok(Err(error)) => Err(format!("source watcher failed: {error}")),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                Err("source watcher stopped unexpectedly".to_owned())
            }
        }
    }

    fn cleanup(&self) {
        let Ok(mut resources) = self.resources.lock() else {
            return;
        };
        resources.startup_cancelled = true;
        terminate_child(&mut resources.host_build);
        terminate_child(&mut resources.guest_build);
        resources.watcher = None;
        resources.changes = None;
        resources.client = None;
        resources.android_session = None;
        resources.android_route = None;
        for child in &mut resources.children {
            let _ = child.kill();
            let _ = child.wait();
        }
        resources.children.clear();
        resources.announcements = None;
    }
}
