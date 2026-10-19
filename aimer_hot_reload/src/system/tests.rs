use std::collections::VecDeque;
use std::fs;

use aimer_reload_protocol::{ReloadResult, ReloadStage};
use tempfile::tempdir;

use crate::config::{ApplicationRuntime, BuildProfile, ReloadPolicy};

use super::*;

#[test]
fn reload_client_uses_the_exact_session_protocol_policy() {
    let config = DevelopmentHostConfig::cli_safe_profile()
        .max_module_bytes(12_345_678)
        .protocol_max_chunk_bytes(12_345)
        .protocol_max_diagnostic_bytes(2_345)
        .protocol_max_terminal_results(3)
        .protocol_io_timeout_ms(4_567);

    let limits = client_protocol_limits(config);

    assert_eq!(limits.max_module_bytes(), 12_345_678);
    assert_eq!(limits.chunk_bytes_limit(), 12_345);
    assert_eq!(limits.diagnostic_bytes_limit(), 2_345);
    assert_eq!(limits.terminal_result_limit(), 3);
    assert_eq!(limits.io_timeout(), Duration::from_millis(4_567));
}

#[derive(Default)]
struct FakeRuntime {
    calls: Mutex<Vec<String>>,
    changes: Mutex<VecDeque<Option<Vec<PathBuf>>>>,
    results: Mutex<VecDeque<Result<ReloadResult, String>>>,
    recovered: Mutex<Option<ReloadResult>>,
    shutdown: Mutex<Option<Arc<AtomicBool>>>,
}

impl FakeRuntime {
    fn record(&self, call: impl Into<String>) {
        self.calls.lock().unwrap().push(call.into());
    }
}

impl SystemRuntime for FakeRuntime {
    fn build_host(&self, _: &Path, _: &Path, _: &str, device: &SystemDevice) -> Result<(), String> {
        self.record(format!("build host {}", device.family.name()));
        Ok(())
    }

    fn build_guest(&self, _: &Path, _: &Path, _: &str, build: u64) -> Result<Vec<u8>, String> {
        self.record(format!("build guest {build}"));
        Ok(vec![0, 97, 115, 109, build as u8])
    }

    fn assemble(
        &self,
        root: &Path,
        _: &Path,
        package: &str,
        device: &SystemDevice,
    ) -> Result<AssembledHost, String> {
        self.record("assemble");
        Ok(match device.family {
            TargetFamily::Macos | TargetFamily::Windows | TargetFamily::Linux => {
                AssembledHost::DesktopExecutable(root.join(package))
            }
            _ => AssembledHost::MobileApplication {
                bundle_id: package.to_owned(),
            },
        })
    }

    fn prepare_route(&self, _: &SystemDevice, port: u16) -> Result<(), String> {
        self.record(format!("route {port}"));
        Ok(())
    }

    fn launch(
        &self,
        _: &SystemDevice,
        _: &str,
        _: &AssembledHost,
        session: &DevelopmentSession,
    ) -> Result<(), String> {
        self.record(format!("launch port={}", session.listener_port()));
        Ok(())
    }

    fn connect(&self, _: &SystemDevice, session: &DevelopmentSession) -> Result<(), String> {
        self.record(format!("connect {}", session.listener_port()));
        Ok(())
    }

    fn push_reload(
        &self,
        request: u64,
        _: ModuleMetadata,
        _: &[u8],
    ) -> Result<ReloadResult, String> {
        self.record(format!("push {request}"));
        self.results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(committed(request)))
    }

    fn recover_result(&self, request: u64) -> Result<Option<ReloadResult>, String> {
        self.record(format!("recover {request}"));
        Ok(self.recovered.lock().unwrap().clone())
    }

    fn start_watcher(&self, _: &Path, _: &[PathBuf]) -> Result<(), String> {
        self.record("watch");
        Ok(())
    }

    fn next_change(&self, _: Duration) -> Result<Option<Vec<PathBuf>>, String> {
        let mut changes = self.changes.lock().unwrap();
        let change = changes.pop_front().flatten();
        if change.is_some()
            && changes.is_empty()
            && let Some(shutdown) = self.shutdown.lock().unwrap().as_ref()
        {
            shutdown.store(true, Ordering::Release);
        }
        Ok(change)
    }

    fn cleanup(&self) {
        self.record("cleanup");
    }
}

fn committed(request: u64) -> ReloadResult {
    ReloadResult::Committed {
        active_generation: request,
        reset_state_entries: 0,
        cleanup_warnings: 0,
    }
}

fn policy() -> ExecutionPolicy {
    ExecutionPolicy::new(
        BuildProfile::Debug,
        ApplicationRuntime::Wasmi,
        ReloadPolicy::HotReload,
    )
    .unwrap()
}

fn fixture(
    runtime: Arc<FakeRuntime>,
) -> (tempfile::TempDir, SystemPipelineOperations<FakeRuntime>) {
    let root = tempdir().unwrap();
    fs::write(root.path().join("Cargo.toml"), "[workspace]\n").unwrap();
    let operations = SystemPipelineOperations::with_runtime(
        root.path(),
        root.path(),
        policy(),
        SystemDevice::new("local", TargetFamily::Linux, "linux"),
        "counter",
        runtime,
    );
    (root, operations)
}

#[cfg(feature = "hot-reload")]
#[test]
fn startup_owns_initial_commit_and_cleanup_without_exposing_session_values() {
    let runtime = Arc::new(FakeRuntime::default());
    let (_root, mut operations) = fixture(Arc::clone(&runtime));

    operations.resolve().unwrap();
    operations.create_session().unwrap();
    operations.build_host().unwrap();
    operations.build_initial_guest().unwrap();
    operations.assemble().unwrap();
    operations.prepare_route().unwrap();
    operations.launch_app().unwrap();
    operations.discover_and_authenticate().unwrap();
    operations.push_initial_module().unwrap();
    operations.cleanup();
    operations.cleanup();

    assert_eq!(runtime.calls.lock().unwrap().last().unwrap(), "cleanup");
    assert_eq!(
        runtime
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| *call == "cleanup")
            .count(),
        1
    );
    assert!(matches!(
        operations.statuses().last(),
        Some(ReloadStatus::Terminal(ReloadResult::Committed { .. }))
    ));
}

#[test]
fn failed_startup_cleanup_releases_a_session_before_any_generation_commits() {
    let runtime = Arc::new(FakeRuntime::default());
    let (_root, mut operations) = fixture(Arc::clone(&runtime));

    operations.resolve().unwrap();
    operations.create_session().unwrap();
    operations.cleanup();
    drop(operations);

    let calls = runtime.calls.lock().unwrap();
    assert_eq!(calls.iter().filter(|call| *call == "cleanup").count(), 1);
}

#[test]
fn a_disconnect_recovers_the_terminal_result_before_rebuilding_again() {
    let runtime = Arc::new(FakeRuntime::default());
    runtime
        .results
        .lock()
        .unwrap()
        .push_back(Err("connection reset".to_owned()));
    *runtime.recovered.lock().unwrap() = Some(committed(1));
    let (_root, mut operations) = fixture(Arc::clone(&runtime));
    operations.build_initial_guest().unwrap();

    operations.push_initial_module().unwrap();

    assert_eq!(
        runtime.calls.lock().unwrap().as_slice(),
        ["build guest 1", "push 1", "recover 1"]
    );
}

#[test]
fn watch_rebuilds_once_for_a_change_and_stops_on_the_shared_signal() {
    let runtime = Arc::new(FakeRuntime::default());
    let root = tempdir().unwrap();
    fs::write(root.path().join("Cargo.toml"), "[workspace]\n").unwrap();
    runtime
        .changes
        .lock()
        .unwrap()
        .push_back(Some(vec![root.path().join("src/page.rs")]));
    let shutdown = Arc::new(AtomicBool::new(false));
    *runtime.shutdown.lock().unwrap() = Some(Arc::clone(&shutdown));
    let mut operations = SystemPipelineOperations::with_runtime(
        root.path(),
        root.path(),
        policy(),
        SystemDevice::new("local", TargetFamily::Linux, "linux"),
        "counter",
        Arc::clone(&runtime),
    )
    .shutdown_signal(shutdown);

    operations.watch().unwrap();

    assert_eq!(
        runtime.calls.lock().unwrap().as_slice(),
        ["watch", "build guest 1", "push 1"]
    );
}

#[test]
fn watch_retains_the_app_after_failure_and_accepts_a_later_edit() {
    let runtime = Arc::new(FakeRuntime::default());
    runtime.results.lock().unwrap().extend([
        Ok(committed(1)),
        Err("guest compilation failed".to_owned()),
        Ok(committed(3)),
    ]);
    let (root, mut operations) = fixture(Arc::clone(&runtime));
    let changed = root.path().join("src/page.rs");
    runtime
        .changes
        .lock()
        .unwrap()
        .extend([Some(vec![changed.clone()]), Some(vec![changed])]);
    let shutdown = Arc::new(AtomicBool::new(false));
    *runtime.shutdown.lock().unwrap() = Some(Arc::clone(&shutdown));
    operations.shutdown = shutdown;
    operations.build_initial_guest().unwrap();
    operations.push_initial_module().unwrap();

    operations.watch().unwrap();

    let statuses = operations.statuses();
    assert!(statuses.iter().any(|status| matches!(
        status,
        ReloadStatus::RecoverableFailure { diagnostic }
            if diagnostic.contains("guest compilation failed")
    )));
    assert!(matches!(
        statuses.last(),
        Some(ReloadStatus::Terminal(ReloadResult::Committed {
            active_generation: 3,
            ..
        }))
    ));
}

#[test]
fn watch_rejection_retains_the_last_generation_and_recovers_on_the_next_edit() {
    let runtime = Arc::new(FakeRuntime::default());
    runtime.results.lock().unwrap().extend([
        Ok(committed(1)),
        Ok(ReloadResult::Rejected {
            stage: ReloadStage::Validation,
            error_code: 17,
            active_generation: 1,
            diagnostic: "unknown required widget".to_owned(),
        }),
        Ok(committed(3)),
    ]);
    let (root, mut operations) = fixture(Arc::clone(&runtime));
    let first_change = root.path().join("src/page.rs");
    let second_change = root.path().join("src/other.rs");
    runtime
        .changes
        .lock()
        .unwrap()
        .extend([Some(vec![first_change]), Some(vec![second_change])]);
    let shutdown = Arc::new(AtomicBool::new(false));
    *runtime.shutdown.lock().unwrap() = Some(Arc::clone(&shutdown));
    operations.shutdown = shutdown;
    operations.build_initial_guest().unwrap();
    operations.push_initial_module().unwrap();

    operations.watch().unwrap();

    let terminal = operations
        .statuses()
        .into_iter()
        .filter_map(|status| match status {
            ReloadStatus::Terminal(result) => Some(result),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(terminal.len(), 3);
    assert!(matches!(
        terminal[1],
        ReloadResult::Rejected {
            active_generation: 1,
            ..
        }
    ));
    assert!(matches!(
        terminal[2],
        ReloadResult::Committed {
            active_generation: 3,
            ..
        }
    ));
}

struct StartupOverlapRuntime {
    guest_started: AtomicBool,
    guest_finished: AtomicBool,
    host_saw_guest_running: AtomicBool,
    release_guest: AtomicBool,
    cancelled: AtomicBool,
    cancel_called: AtomicBool,
    fail_guest: AtomicBool,
}

impl StartupOverlapRuntime {
    fn new() -> Self {
        Self {
            guest_started: AtomicBool::new(false),
            guest_finished: AtomicBool::new(false),
            host_saw_guest_running: AtomicBool::new(false),
            release_guest: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            cancel_called: AtomicBool::new(false),
            fail_guest: AtomicBool::new(false),
        }
    }

    fn wait_for_guest_start(&self) {
        while !self.guest_started.load(Ordering::Acquire)
            && !self.release_guest.load(Ordering::Acquire)
        {
            thread::yield_now();
        }
    }
}

impl SystemRuntime for StartupOverlapRuntime {
    fn build_host(&self, _: &Path, _: &Path, _: &str, _: &SystemDevice) -> Result<(), String> {
        self.wait_for_guest_start();
        if self.guest_started.load(Ordering::Acquire)
            && !self.guest_finished.load(Ordering::Acquire)
        {
            self.host_saw_guest_running.store(true, Ordering::Release);
        }
        if self.cancelled.load(Ordering::Acquire) {
            Err("native build cancelled".to_owned())
        } else {
            Ok(())
        }
    }

    fn build_guest(&self, _: &Path, _: &Path, _: &str, _: u64) -> Result<Vec<u8>, String> {
        self.guest_started.store(true, Ordering::Release);
        if self.fail_guest.load(Ordering::Acquire) {
            return Err("guest compilation failed".to_owned());
        }
        while !self.release_guest.load(Ordering::Acquire) && !self.cancelled.load(Ordering::Acquire)
        {
            thread::yield_now();
        }
        if self.cancelled.load(Ordering::Acquire) {
            return Err("guest build cancelled".to_owned());
        }
        self.guest_finished.store(true, Ordering::Release);
        Ok(vec![0, 97, 115, 109, 1])
    }

    fn assemble(
        &self,
        _: &Path,
        _: &Path,
        package: &str,
        _: &SystemDevice,
    ) -> Result<AssembledHost, String> {
        Ok(AssembledHost::DesktopExecutable(PathBuf::from(package)))
    }

    fn prepare_route(&self, _: &SystemDevice, _: u16) -> Result<(), String> {
        Ok(())
    }

    fn launch(
        &self,
        _: &SystemDevice,
        _: &str,
        _: &AssembledHost,
        _: &DevelopmentSession,
    ) -> Result<(), String> {
        Ok(())
    }

    fn connect(&self, _: &SystemDevice, _: &DevelopmentSession) -> Result<(), String> {
        Ok(())
    }

    fn push_reload(
        &self,
        request: u64,
        _: ModuleMetadata,
        _: &[u8],
    ) -> Result<ReloadResult, String> {
        Ok(committed(request))
    }

    fn recover_result(&self, _: u64) -> Result<Option<ReloadResult>, String> {
        Ok(None)
    }

    fn start_watcher(&self, _: &Path, _: &[PathBuf]) -> Result<(), String> {
        Ok(())
    }

    fn next_change(&self, _: Duration) -> Result<Option<Vec<PathBuf>>, String> {
        Ok(None)
    }

    fn cancel_startup(&self) {
        self.cancel_called.store(true, Ordering::Release);
        self.cancelled.store(true, Ordering::Release);
        self.release_guest.store(true, Ordering::Release);
    }

    fn cleanup(&self) {}
}

#[cfg(feature = "hot-reload")]
#[test]

fn initial_guest_build_overlaps_native_build_and_waits_until_initial_push() {
    let runtime = Arc::new(StartupOverlapRuntime::new());
    let root = tempdir().unwrap();
    fs::write(root.path().join("Cargo.toml"), "[workspace]\n").unwrap();
    let mut operations = SystemPipelineOperations::with_runtime(
        root.path(),
        root.path(),
        policy(),
        SystemDevice::new("local", TargetFamily::Linux, "linux"),
        "counter",
        Arc::clone(&runtime),
    );
    operations.create_session().unwrap();

    let release = Arc::clone(&runtime);
    let delayed_release = thread::spawn(move || {
        while !release.guest_started.load(Ordering::Acquire) {
            thread::yield_now();
        }
        thread::sleep(Duration::from_millis(100));
        release.release_guest.store(true, Ordering::Release);
    });

    operations.build_initial_guest().unwrap();
    operations.build_host().unwrap();
    assert!(runtime.host_saw_guest_running.load(Ordering::Acquire));
    assert!(!runtime.guest_finished.load(Ordering::Acquire));

    operations.assemble().unwrap();
    operations.prepare_route().unwrap();
    operations.launch_app().unwrap();
    operations.discover_and_authenticate().unwrap();
    assert!(!runtime.guest_finished.load(Ordering::Acquire));

    runtime.release_guest.store(true, Ordering::Release);
    operations.push_initial_module().unwrap();
    delayed_release.join().unwrap();
    assert!(runtime.guest_finished.load(Ordering::Acquire));
}

#[cfg(feature = "hot-reload")]
#[test]
fn guest_failure_cancels_the_other_initial_startup_branch() {
    let runtime = Arc::new(StartupOverlapRuntime::new());
    runtime.fail_guest.store(true, Ordering::Release);
    let root = tempdir().unwrap();
    fs::write(root.path().join("Cargo.toml"), "[workspace]\n").unwrap();
    let mut operations = SystemPipelineOperations::with_runtime(
        root.path(),
        root.path(),
        policy(),
        SystemDevice::new("local", TargetFamily::Linux, "linux"),
        "counter",
        Arc::clone(&runtime),
    );

    operations.build_initial_guest().unwrap();
    while !runtime.cancel_called.load(Ordering::Acquire) {
        thread::yield_now();
    }
    let error = operations.build_host().unwrap_err();

    assert_eq!(error, "native build cancelled");
    operations.cleanup();
}

#[cfg(feature = "hot-reload")]
#[test]
fn invalid_policy_and_web_target_fail_before_any_side_effect() {
    let runtime = Arc::new(FakeRuntime::default());
    let (_root, mut native) = fixture(Arc::clone(&runtime));
    native.policy = ExecutionPolicy::new(
        BuildProfile::Debug,
        ApplicationRuntime::NativeAot,
        ReloadPolicy::Disabled,
    )
    .unwrap();
    assert!(
        native
            .resolve()
            .unwrap_err()
            .contains("debug/wasmi/hot-reload")
    );

    let (_root, mut web) = fixture(Arc::clone(&runtime));
    web.device.family = TargetFamily::Web;
    assert!(web.resolve().unwrap_err().contains("does not support"));
    assert!(runtime.calls.lock().unwrap().is_empty());
}
