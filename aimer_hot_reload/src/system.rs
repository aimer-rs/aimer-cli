use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use aimer_reload_protocol::{DevelopmentHostConfig, ModuleMetadata, ProtocolLimits, ReloadResult};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use sha2::Digest;

use crate::config::{
    AimerManifest, ApplicationRuntime, BuildProfile, ExecutionPolicy, ReloadPolicy,
};

use super::build::{BuildCommand, HotReloadBuildPlan, load_guest_artifact};
use super::client::{ClientError, ReloadClient};
use super::generation::{
    GuestGenerationMode, GuestPackageSpec, generate_guest_package, prepare_automatic_guest,
};
use super::launch::{LaunchConfiguration, LaunchStep};
use super::pipeline::PipelineOperations;
use super::readiness::await_listener_readiness;
use super::route::{
    AndroidRouteAdapter, DesktopRouteAdapter, IosDeviceRouteAdapter, OwnedCommandGuard,
    RouteReservation, SimulatorRouteAdapter, SystemCommandExecutor,
};
use super::session::DevelopmentSession;
use super::status::ReloadStatus;
use super::targets::TargetFamily;
use super::watch::{ChangeImpact, WatchSet};

mod process;
pub use process::SystemProcessRuntime;

const PROTOCOL_VERSION: (u16, u16) = (1, 0);
const READINESS_TIMEOUT: Duration = Duration::from_secs(30);

/// Device information required by the reusable hot-reload pipeline.
///
/// The command binary's `Device` type is intentionally not part of the reusable
/// `aimer_cli` library. Integration converts the selected device into this
/// value, preserving the exact target and identifier chosen by the user.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemDevice {
    name: String,
    family: TargetFamily,
    id: String,
}

impl SystemDevice {
    /// Creates the device selection consumed by [`SystemPipelineOperations`].
    #[inline]
    pub fn new(name: impl Into<String>, family: TargetFamily, id: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            family,
            id: id.into(),
        }
    }

    /// Returns the selected target family.
    #[inline]
    pub const fn family(&self) -> TargetFamily {
        self.family
    }

    /// Returns the exact platform-tool device identifier.
    #[inline]
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Result of the target-specific native assembly stage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssembledHost {
    /// A local desktop executable that can be started directly.
    DesktopExecutable(PathBuf),
    /// An installed mobile application addressed by its bundle/application id.
    MobileApplication { bundle_id: String },
}

/// Narrow side-effect boundary used by the production pipeline.
///
/// Implementations must retain every process and watcher they create until
/// [`Self::cleanup`] is called. Private launch environment and standard-input
/// values are passed only to `launch`; implementations must never include them
/// in diagnostics.
pub trait SystemRuntime: Send + Sync + 'static {
    fn build_host(
        &self,
        project_root: &Path,
        workspace_root: &Path,
        package: &str,
        device: &SystemDevice,
    ) -> Result<(), String>;

    fn build_guest(
        &self,
        project_root: &Path,
        workspace_root: &Path,
        package: &str,
        build_number: u64,
    ) -> Result<Vec<u8>, String>;

    /// Cancels compiler processes owned by the current startup branch.
    ///
    /// Production implementations must terminate pending native and guest
    /// compiler children and make them observable as cancelled to the caller.
    /// Test and embedded runtimes may keep the default no-op when their build
    /// operations are already bounded.
    fn cancel_startup(&self) {}

    fn assemble(
        &self,
        project_root: &Path,
        workspace_root: &Path,
        package: &str,
        device: &SystemDevice,
    ) -> Result<AssembledHost, String>;

    fn prepare_route(&self, device: &SystemDevice, listener_port: u16) -> Result<(), String>;

    fn launch(
        &self,
        device: &SystemDevice,
        package: &str,
        host: &AssembledHost,
        session: &DevelopmentSession,
    ) -> Result<(), String>;

    fn connect(&self, device: &SystemDevice, session: &DevelopmentSession) -> Result<(), String>;

    fn push_reload(
        &self,
        request_id: u64,
        metadata: ModuleMetadata,
        module: &[u8],
    ) -> Result<ReloadResult, String>;

    fn recover_result(&self, request_id: u64) -> Result<Option<ReloadResult>, String>;

    fn start_watcher(&self, project_root: &Path, ignored: &[PathBuf]) -> Result<(), String>;

    fn next_change(&self, timeout: Duration) -> Result<Option<Vec<PathBuf>>, String>;

    fn cleanup(&self);
}

/// Concrete owner of one native hot-reload run.
///
/// Construction is side-effect free. The existing pipeline driver invokes the
/// methods in `PipelineOperations` order; this value retains the initial guest
/// build handle, target route, launched app, watcher, and request recovery
/// state until the driver's mandatory cleanup call.
pub struct SystemPipelineOperations<R: SystemRuntime = SystemProcessRuntime> {
    project_root: PathBuf,
    workspace_root: PathBuf,
    policy: ExecutionPolicy,
    device: SystemDevice,
    package: String,
    runtime: Arc<R>,
    shutdown: Arc<AtomicBool>,
    statuses: Arc<Mutex<Vec<ReloadStatus>>>,
    status_output: Arc<dyn Fn(&ReloadStatus) + Send + Sync>,
    watch_set: WatchSet,
    listener_port: u16,
    widget_ir_diagnostics: bool,
    session: Option<DevelopmentSession>,
    assembled_host: Option<AssembledHost>,
    initial_guest_build: Option<JoinHandle<Result<Vec<u8>, String>>>,
    request_id: u64,
    build_number: u64,
    cleaned: bool,
}

impl SystemPipelineOperations<SystemProcessRuntime> {
    /// Creates the production operations owner from the run command's resolved inputs.
    ///
    /// The constructor starts no process and creates no credential. Side effects
    /// begin only when the pipeline driver calls `resolve` and subsequent stages.
    #[inline]
    pub fn new(
        project_root: impl Into<PathBuf>,
        workspace_root: impl Into<PathBuf>,
        policy: ExecutionPolicy,
        device: SystemDevice,
        package: impl Into<String>,
    ) -> Self {
        Self::with_runtime(
            project_root,
            workspace_root,
            policy,
            device,
            package,
            Arc::new(SystemProcessRuntime::new()),
        )
        .status_output(|status| eprintln!("{status}"))
    }
}

impl<R> SystemPipelineOperations<R>
where
    R: SystemRuntime,
{
    /// Creates a pipeline from resolved run-command inputs and a runtime.
    #[inline]
    pub fn with_runtime(
        project_root: impl Into<PathBuf>,
        workspace_root: impl Into<PathBuf>,
        policy: ExecutionPolicy,
        device: SystemDevice,
        package: impl Into<String>,
        runtime: Arc<R>,
    ) -> Self {
        let project_root = project_root.into();
        let workspace_root = workspace_root.into();
        Self {
            watch_set: WatchSet::automatic(
                project_root.clone(),
                portable_source_root(&project_root),
                local_path_dependencies(&project_root),
                vec![
                    workspace_root.join("target"),
                    project_root.join("target"),
                    project_root.join("builds"),
                ],
            ),
            project_root,
            workspace_root,
            policy,
            device,
            package: package.into(),
            runtime,
            shutdown: Arc::new(AtomicBool::new(false)),
            statuses: Arc::new(Mutex::new(Vec::new())),
            status_output: Arc::new(|_| {}),
            listener_port: 0,
            widget_ir_diagnostics: false,
            session: None,
            assembled_host: None,
            initial_guest_build: None,
            request_id: 0,
            build_number: 0,
            cleaned: false,
        }
    }

    /// Supplies the shared cancellation flag observed by the watch loop.
    #[inline]
    pub fn shutdown_signal(mut self, shutdown: Arc<AtomicBool>) -> Self {
        self.shutdown = shutdown;
        self
    }

    /// Enables verbose Widget IR stage diagnostics in the launched native host.
    #[inline]
    pub fn widget_ir_diagnostics(mut self, enabled: bool) -> Self {
        self.widget_ir_diagnostics = enabled;
        self
    }

    /// Replaces the source classification resolved from Cargo metadata.
    #[inline]
    pub fn watch_set(mut self, watch_set: WatchSet) -> Self {
        self.watch_set = watch_set;
        self
    }

    /// Supplies the secret-free destination for progress and terminal statuses.
    #[inline]
    pub fn status_output(mut self, output: impl Fn(&ReloadStatus) + Send + Sync + 'static) -> Self {
        self.status_output = Arc::new(output);
        self
    }

    /// Returns the secret-free status history emitted by this run.
    pub fn statuses(&self) -> Vec<ReloadStatus> {
        self.statuses.lock().expect("status lock poisoned").clone()
    }

    fn status(&self, status: ReloadStatus) {
        (self.status_output)(&status);
        self.statuses
            .lock()
            .expect("status lock poisoned")
            .push(status);
    }

    fn rebuild_and_push(&mut self) -> Result<(), String> {
        self.build_number = self.build_number.checked_add(1).ok_or_else(|| {
            "hot-reload build counter exhausted; restart the development session".to_owned()
        })?;
        self.status(ReloadStatus::Compiling {
            build: self.build_number,
        });
        let module = self.runtime.build_guest(
            &self.project_root,
            &self.workspace_root,
            &self.package,
            self.build_number,
        )?;
        self.push(module)
    }

    fn push(&mut self, module: Vec<u8>) -> Result<(), String> {
        self.request_id = self.request_id.checked_add(1).ok_or_else(|| {
            "hot-reload request counter exhausted; restart the development session".to_owned()
        })?;
        let request_id = self.request_id;
        self.status(ReloadStatus::Uploading {
            sent: module.len(),
            total: module.len(),
        });
        self.status(ReloadStatus::WaitingForCommit {
            request: request_id,
        });
        let metadata = ModuleMetadata::new(
            stable_identity(&self.package),
            build_identity(&module),
            1,
            0,
            empty_capability_digest(),
        );
        let result = match self.runtime.push_reload(request_id, metadata, &module) {
            Ok(result) => result,
            Err(disconnected) => self
                .runtime
                .recover_result(request_id)?
                .ok_or(disconnected)?,
        };
        let committed = matches!(result, ReloadResult::Committed { .. });
        self.status(ReloadStatus::Terminal(result));
        if !committed && self.build_number == 1 {
            return Err("the initial guest generation was not committed".to_owned());
        }
        Ok(())
    }

    fn finish_initial_guest_build(&mut self) -> Result<Vec<u8>, String> {
        let handle = self
            .initial_guest_build
            .take()
            .ok_or_else(|| "initial guest build was not started".to_owned())?;
        match handle.join() {
            Ok(result) => result,
            Err(_) => {
                self.runtime.cancel_startup();
                Err("initial guest build thread panicked".to_owned())
            }
        }
    }

    fn cancel_initial_guest_build(&mut self) {
        if let Some(handle) = self.initial_guest_build.take() {
            self.runtime.cancel_startup();
            let _ = handle.join();
        }
    }
}

impl<R> Drop for SystemPipelineOperations<R>
where
    R: SystemRuntime,
{
    fn drop(&mut self) {
        if !self.cleaned {
            self.cleaned = true;
            self.cancel_initial_guest_build();
            self.runtime.cleanup();
            self.assembled_host = None;
            self.session = None;
        }
    }
}

impl<R> PipelineOperations for SystemPipelineOperations<R>
where
    R: SystemRuntime,
{
    fn resolve(&mut self) -> Result<(), String> {
        if self.policy.profile() != BuildProfile::Debug
            || self.policy.runtime() != ApplicationRuntime::Wasmi
            || self.policy.reload() != ReloadPolicy::HotReload
        {
            return Err(
                "the system hot-reload pipeline requires debug/wasmi/hot-reload".to_owned(),
            );
        }
        if !self.device.family.supports_hot_reload() {
            return Err(format!(
                "{} does not support native hot reload",
                self.device.family
            ));
        }
        if self.package.trim().is_empty() {
            return Err("the hot-reload host package name is empty".to_owned());
        }
        if !self.project_root.join("Cargo.toml").is_file() {
            return Err(format!(
                "project manifest is missing at {}",
                self.project_root.join("Cargo.toml").display()
            ));
        }
        if !self.workspace_root.join("Cargo.toml").is_file() {
            return Err(format!(
                "workspace manifest is missing at {}",
                self.workspace_root.join("Cargo.toml").display()
            ));
        }
        Ok(())
    }

    fn create_session(&mut self) -> Result<(), String> {
        let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| format!("failed to allocate a reload listener port: {error}"))?;
        self.listener_port = reservation
            .local_addr()
            .map_err(|error| format!("failed to inspect the reload listener port: {error}"))?
            .port();
        drop(reservation);
        self.session = DevelopmentSession::for_policy(self.policy, self.listener_port)
            .map_err(|error| format!("failed to create the development session: {error}"))?
            .map(|session| session.widget_ir_diagnostics(self.widget_ir_diagnostics));
        if self.session.is_none() {
            return Err("the hot-reload policy did not create a development session".to_owned());
        }
        Ok(())
    }

    fn build_host(&mut self) -> Result<(), String> {
        self.runtime.build_host(
            &self.project_root,
            &self.workspace_root,
            &self.package,
            &self.device,
        )
    }

    fn build_initial_guest(&mut self) -> Result<(), String> {
        if self.initial_guest_build.is_some() {
            return Err("the initial guest build was already started".to_owned());
        }
        self.build_number = 1;
        self.status(ReloadStatus::Compiling { build: 1 });
        let runtime = Arc::clone(&self.runtime);
        let project_root = self.project_root.clone();
        let workspace_root = self.workspace_root.clone();
        let package = self.package.clone();
        self.initial_guest_build = Some(
            thread::Builder::new()
                .name("aimer-hot-reload-guest-build".to_owned())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        runtime.build_guest(&project_root, &workspace_root, &package, 1)
                    }));
                    match result {
                        Ok(Ok(module)) => Ok(module),
                        Ok(Err(error)) => {
                            runtime.cancel_startup();
                            Err(error)
                        }
                        Err(_) => {
                            runtime.cancel_startup();
                            Err("initial guest build panicked".to_owned())
                        }
                    }
                })
                .map_err(|error| format!("failed to start initial guest build: {error}"))?,
        );
        Ok(())
    }

    fn assemble(&mut self) -> Result<(), String> {
        self.assembled_host = Some(self.runtime.assemble(
            &self.project_root,
            &self.workspace_root,
            &self.package,
            &self.device,
        )?);
        Ok(())
    }

    fn prepare_route(&mut self) -> Result<(), String> {
        self.runtime.prepare_route(&self.device, self.listener_port)
    }

    fn launch_app(&mut self) -> Result<(), String> {
        let host = self
            .assembled_host
            .as_ref()
            .ok_or_else(|| "native host was not assembled before launch".to_owned())?;
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| "development session was not created before launch".to_owned())?;
        self.runtime
            .launch(&self.device, &self.package, host, session)
    }

    fn discover_and_authenticate(&mut self) -> Result<(), String> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| "development session was not created before discovery".to_owned())?;
        self.runtime.connect(&self.device, session)
    }

    fn push_initial_module(&mut self) -> Result<(), String> {
        let module = self.finish_initial_guest_build()?;
        self.push(module)
    }

    fn watch(&mut self) -> Result<(), String> {
        self.runtime
            .start_watcher(&self.project_root, self.watch_set.watch_roots())?;
        while !self.shutdown.load(Ordering::Acquire) {
            let Some(paths) = self.runtime.next_change(Duration::from_millis(50))? else {
                continue;
            };
            match self.watch_set.classify(paths.iter().map(PathBuf::as_path)) {
                ChangeImpact::Ignored => {}
                ChangeImpact::RebuildGuest => {
                    if let Err(diagnostic) = self.rebuild_and_push() {
                        self.status(ReloadStatus::RecoverableFailure { diagnostic });
                    }
                }
                ChangeImpact::RestartNativeHost => {
                    self.status(ReloadStatus::NativeRestartRequired {
                        reason: "native host source changed".to_owned(),
                    });
                }
            }
        }
        Ok(())
    }

    fn cleanup(&mut self) {
        if !self.cleaned {
            self.cleaned = true;
            self.cancel_initial_guest_build();
            self.runtime.cleanup();
            self.assembled_host = None;
            self.session = None;
        }
    }
}

fn terminate_child(child: &mut Option<Child>) {
    if let Some(mut child) = child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn stream_lines<R>(reader: R, sender: mpsc::Sender<String>)
where
    R: std::io::Read + Send + 'static,
{
    thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            eprintln!("{line}");
            let _ = sender.send(line);
        }
    });
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> String {
    "hot-reload runtime state lock was poisoned".to_owned()
}

fn client_error(error: ClientError) -> String {
    error.to_string()
}

fn client_protocol_limits(config: DevelopmentHostConfig) -> ProtocolLimits {
    ProtocolLimits::new(
        config.module_bytes_limit(),
        Duration::from_millis(config.protocol_io_timeout_ms_limit()),
    )
    .max_chunk_bytes(config.protocol_chunk_bytes_limit())
    .max_diagnostic_bytes(config.protocol_diagnostic_bytes_limit())
    .max_terminal_results(config.protocol_terminal_result_limit())
}

fn mobile_application_id(project_root: &Path, fallback: &str) -> String {
    AimerManifest::load_from(project_root)
        .ok()
        .flatten()
        .map(|manifest| {
            let group = manifest.package.group.trim_end_matches('.');
            if group.is_empty() || group.ends_with(fallback) {
                if group.is_empty() {
                    fallback.to_owned()
                } else {
                    group.to_owned()
                }
            } else {
                format!("{group}.{fallback}")
            }
        })
        .unwrap_or_else(|| fallback.to_owned())
}

fn android_application_id(project_root: &Path, fallback: &str) -> String {
    fs::read_to_string(project_root.join("builds/android/app/build.gradle.kts"))
        .ok()
        .and_then(|script| {
            script
                .lines()
                .find(|line| line.contains("applicationId"))
                .and_then(|line| line.split('"').nth(1))
                .map(str::to_owned)
        })
        .unwrap_or_else(|| mobile_application_id(project_root, fallback))
}

fn portable_source_root(project_root: &Path) -> PathBuf {
    let manifest = fs::read_to_string(project_root.join("Cargo.toml"))
        .ok()
        .and_then(|source| toml::from_str::<toml::Value>(&source).ok());
    let crate_source = manifest
        .as_ref()
        .and_then(|manifest| manifest.get("lib"))
        .and_then(|library| library.get("path"))
        .and_then(toml::Value::as_str)
        .map(|path| project_root.join(path))
        .or_else(|| {
            let library = project_root.join("src/lib.rs");
            library.is_file().then_some(library)
        })
        .unwrap_or_else(|| project_root.join("src/main.rs"));
    crate_source.parent().unwrap_or(project_root).to_owned()
}

fn local_path_dependencies(project_root: &Path) -> Vec<PathBuf> {
    let Some(manifest) = fs::read_to_string(project_root.join("Cargo.toml"))
        .ok()
        .and_then(|source| toml::from_str::<toml::Value>(&source).ok())
    else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    collect_dependency_paths(&manifest, project_root, &mut roots);
    roots.sort();
    roots.dedup();
    roots
}

fn collect_dependency_paths(value: &toml::Value, project_root: &Path, roots: &mut Vec<PathBuf>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(dependencies) = table.get(section).and_then(toml::Value::as_table) {
            for dependency in dependencies.values() {
                if let Some(path) = dependency
                    .as_table()
                    .and_then(|dependency| dependency.get("path"))
                    .and_then(toml::Value::as_str)
                {
                    let path = project_root.join(path);
                    roots.push(fs::canonicalize(&path).unwrap_or(path));
                }
            }
        }
    }
    if let Some(targets) = table.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            collect_dependency_paths(target, project_root, roots);
        }
    }
}

fn stable_identity(package: &str) -> [u8; 16] {
    let digest = sha2::Sha256::digest(format!("AIMER-APPLICATION-V1\0{package}"));
    digest[..16]
        .try_into()
        .expect("SHA-256 prefix has 16 bytes")
}

fn build_identity(module: &[u8]) -> [u8; 16] {
    let digest = sha2::Sha256::digest(module);
    digest[..16]
        .try_into()
        .expect("SHA-256 prefix has 16 bytes")
}

fn empty_capability_digest() -> [u8; 32] {
    use sha2::Digest;

    let mut digest = sha2::Sha256::new();
    digest.update(b"AIMER-CAPABILITY-MANIFEST-V1");
    digest.update(0_u64.to_le_bytes());
    digest.finalize().into()
}

#[cfg(test)]
mod tests;
