use std::io::{BufRead, BufReader};
use std::net::IpAddr;
use std::ops::Deref;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam::channel::Sender;

use crate::commands::run::Device;
use crate::commands::run::android::AndroidRunner;
use crate::commands::run::capability_sources::configure_command;
use crate::commands::run::cargo_build::{CargoBuildTarget, cargo_command, wait_for_child};
use crate::commands::run::console::{RunnerEvent, Status};
use crate::commands::run::desktop::DesktopRunner;
use crate::commands::run::helpers::set_status;
use crate::commands::run::ios::IosRunner;
use crate::commands::run::macos::MacosRunner;
use crate::commands::run::web::WebRunner;
use crate::session::SessionHandle;
use crate::targets::Targets;

/// Everything a per-target runner needs to build and launch the app.
pub struct RunContext {
    pub device: Device,
    pub pkg_name: String,
    pub tx: Sender<RunnerEvent>,
    pub current_child: Arc<Mutex<Option<Child>>>,
    pub inspector_address: IpAddr,
    pub inspector_port: u16,
    /// Compile and package with the release profile instead of debug, as asked
    /// for by `aimer run --release`. Every stage derives its profile from this
    /// flag: the cargo profile, the Xcode configuration, the Gradle task and
    /// therefore the artifact paths.
    pub release: bool,
    /// Optional control session for process identity publication and agent
    /// restart/stop commands.
    pub session: Option<SessionHandle>,
    /// Set when the console has superseded this run (a restart or a quit).
    /// [`drive`] stops at the next stage boundary instead of carrying on beside
    /// the run that replaced it.
    pub cancelled: Arc<AtomicBool>,
}

impl RunContext {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Stop a superseded run: whatever it has spawned since the console last
    /// killed its child must not outlive it.
    fn abandon(&self) {
        if let Some(child) = self.current_child.lock().unwrap().take() {
            terminate(child);
        }
    }
}

/// The console's handle on a run started by `spawn_runner`.
///
/// It dereferences to the slot holding the run's current child process, so it
/// can be used wherever that slot was. [`stop`](Self::stop) is the complete
/// way to end the run: killing the child alone leaves the pipeline thread free
/// to start its next stage next to the run that replaces it.
pub struct RunnerHandle {
    current_child: Arc<Mutex<Option<Child>>>,
    cancelled: Arc<AtomicBool>,
}

impl RunnerHandle {
    pub(crate) fn new(current_child: Arc<Mutex<Option<Child>>>, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            current_child,
            cancelled,
        }
    }

    /// Cancel the run and kill the process it currently has registered.
    pub fn stop(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(child) = self.current_child.lock().unwrap().take() {
            terminate(child);
        }
    }
}

/// How long a child gets to exit after SIGTERM before it is killed outright.
const TERMINATE_GRACE: Duration = Duration::from_secs(3);

/// Stop `child` the way Ctrl-C in a terminal would, then reap it.
///
/// The process the console tracks is often only a client: `xcrun simctl launch`
/// and `xcrun devicectl device process launch --console` run the app elsewhere
/// and forward the signals they can catch to it. SIGKILL cannot be caught, so
/// killing the client would leave the app running on the simulator or phone
/// with its console gone, and the next launch would attach to that stale
/// process instead of starting the new build. SIGTERM lets the client take the
/// app down with it; SIGKILL is only the fallback.
///
/// The signal is sent before this returns. Waiting for the exit, escalating
/// and reaping happen on a helper thread so a slow client cannot stall the
/// console.
pub(crate) fn terminate(mut child: Child) {
    #[cfg(unix)]
    {
        // SAFETY: `kill(2)` only reads its arguments; the pid belongs to a
        // child we have not yet reaped, so it cannot have been recycled.
        let signalled = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) } == 0;
        if signalled {
            thread::spawn(move || {
                let deadline = Instant::now() + TERMINATE_GRACE;
                while Instant::now() < deadline {
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                let _ = child.kill();
                let _ = child.wait();
            });
            return;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

impl Deref for RunnerHandle {
    type Target = Mutex<Option<Child>>;

    fn deref(&self) -> &Self::Target {
        &self.current_child
    }
}

/// One step of the unified run pipeline, in the order [`drive`] executes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Compile the Rust library or binary for the target.
    Build,
    /// Turn the compiled artifact into something launchable: stage the native
    /// library and the registered assets, then run the platform packager
    /// (`xcodebuild`, Gradle, ...).
    Assemble,
    /// Install the bundle if the platform needs it and start the app, leaving
    /// the process registered as the current child so it can be killed and its
    /// output streamed.
    Launch,
}

impl Stage {
    /// The stages in pipeline order.
    pub const ALL: [Stage; 3] = [Stage::Build, Stage::Assemble, Stage::Launch];
}

/// Whether the pipeline should carry on after a stage.
///
/// A stage that returns [`Flow::Abort`] has already reported the failure to the
/// console — usually through [`fail`](crate::commands::run::helpers::fail) — so
/// [`drive`] just stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum Flow {
    /// The stage succeeded; run the next one.
    Continue,
    /// The stage failed or was cancelled; stop the pipeline.
    Abort,
}

/// A per-target run pipeline, split into the three stages every platform shares:
/// [`build`](Runner::build) → [`assemble`](Runner::assemble) →
/// [`launch`](Runner::launch).
///
/// Implementors only describe *what* their platform does in each stage;
/// sequencing, the `Running`/`Idling` status transitions and waiting for the app
/// to exit all live in [`drive`], so no runner repeats them. State discovered in
/// one stage — an Android ABI, an iOS bundle id — is kept in the runner itself,
/// which is why the stages take `&mut self`.
///
/// `Send` is required because runners are dispatched onto a background thread.
pub trait Runner: Send {
    /// Compile the Rust code for the target.
    ///
    /// Defaults to doing nothing, for targets whose launch step compiles as a
    /// side effect (`trunk serve` on web).
    fn build(&mut self, _ctx: &RunContext) -> Flow {
        Flow::Continue
    }

    /// Package the compiled artifact into a launchable bundle.
    ///
    /// Defaults to doing nothing, for targets that launch the compiler output
    /// directly.
    fn assemble(&mut self, _ctx: &RunContext) -> Flow {
        Flow::Continue
    }

    /// Start the app, registering the spawned process as the current child.
    ///
    /// The stage must not wait for the app to exit — [`drive`] does that once
    /// the process is registered.
    fn launch(&mut self, ctx: &RunContext) -> Flow;

    /// Dispatch `stage` to the matching method. Provided so [`drive`] can walk
    /// [`Stage::ALL`]; not meant to be overridden.
    fn stage(&mut self, ctx: &RunContext, stage: Stage) -> Flow {
        match stage {
            Stage::Build => self.build(ctx),
            Stage::Assemble => self.assemble(ctx),
            Stage::Launch => self.launch(ctx),
        }
    }
}

/// Drive `runner` through the unified pipeline: build → assemble → launch.
///
/// Stops at the first stage that returns [`Flow::Abort`], leaving the status the
/// failing stage reported. Otherwise the app is running, so the status becomes
/// [`Status::Running`], and once the launched process exits the pipeline settles
/// back to [`Status::Idling`].
pub fn drive(mut runner: Box<dyn Runner>, ctx: RunContext) {
    for stage in Stage::ALL {
        if ctx.is_cancelled() {
            return ctx.abandon();
        }
        if runner.stage(&ctx, stage) == Flow::Abort {
            return;
        }
    }
    if ctx.is_cancelled() {
        return ctx.abandon();
    }

    set_status(&ctx.tx, Status::Running);
    if let Some(session) = &ctx.session {
        session.set_status(crate::session::SessionStatus::Running, None);
    }
    wait_for_child(&ctx.current_child);
    if ctx.is_cancelled() {
        return;
    }
    set_status(&ctx.tx, Status::Idling);
    if let Some(session) = &ctx.session {
        session.set_status(crate::session::SessionStatus::Idling, None);
    }
}

/// Resolve the [`Runner`] for a target, or `None` if the target is not
/// runnable on the fly.
pub fn runner_for(target: Targets) -> Option<Box<dyn Runner>> {
    match target {
        Targets::Macos => Some(Box::new(MacosRunner::new())),
        Targets::Web => Some(Box::new(WebRunner::new())),
        Targets::Ios => Some(Box::new(IosRunner::device())),
        Targets::IosSimulator => Some(Box::new(IosRunner::simulator())),
        Targets::Android | Targets::AndroidSimulator => Some(Box::new(AndroidRunner::new())),
        Targets::Windows => Some(Box::new(DesktopRunner::new(Targets::Windows))),
        Targets::Linux => Some(Box::new(DesktopRunner::new(Targets::Linux))),
        _ => None,
    }
}

/// Shared wasm-pack web build used by both the initial run and hot reloads.
///
/// Spawns the build on a background thread and streams its stdout/stderr back
/// as [`RunnerEvent`]s, de-duplicating what used to be two copies of this
/// logic inside `console.rs`. `release` selects the cargo profile wasm-pack
/// compiles with, mirroring `aimer run --release`.
pub fn spawn_wasm_pack(tx: Sender<RunnerEvent>, release: bool) {
    thread::spawn(move || {
        let _ = tx.send(RunnerEvent::StatusChange(Status::Compiling(0)));
        let _ = tx.send(RunnerEvent::BuildLog(
            "Running wasm-pack build...".to_string(),
        ));

        let mut command = cargo_command(&CargoBuildTarget::Web, release);
        if let Err(error) = configure_command(&mut command) {
            let _ = tx.send(RunnerEvent::BuildLog(error));
            let _ = tx.send(RunnerEvent::StatusChange(Status::Error));
            return;
        }
        let mut wasm_build = match command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                let _ = tx.send(RunnerEvent::BuildLog(format!(
                    "Failed to start wasm-pack: {e}"
                )));
                let _ = tx.send(RunnerEvent::StatusChange(Status::Error));
                return;
            }
        };

        if let Some(stdout) = wasm_build.stdout.take() {
            let tx_out = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    let _ = tx_out.send(RunnerEvent::BuildLog(line));
                }
            });
        }

        if let Some(stderr) = wasm_build.stderr.take() {
            let tx_err = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                let mut compile_count = 0;
                for line in reader.lines().map_while(Result::ok) {
                    if line.contains("Compiling") {
                        compile_count = (compile_count + 5).min(99);
                        let _ = tx_err
                            .send(RunnerEvent::StatusChange(Status::Compiling(compile_count)));
                    } else if line.contains("Finished") {
                        let _ = tx_err.send(RunnerEvent::StatusChange(Status::Compiling(100)));
                    }
                    let _ = tx_err.send(RunnerEvent::BuildLog(line));
                }
            });
        }

        match wasm_build.wait() {
            Ok(status) if status.success() => {
                let _ = tx.send(RunnerEvent::BuildLog(
                    "wasm-pack build successful. Vite will auto-reload.".to_string(),
                ));
            }
            _ => {
                let _ = tx.send(RunnerEvent::BuildLog("wasm-pack build failed.".to_string()));
            }
        }
        let _ = tx.send(RunnerEvent::StatusChange(Status::Running));
    });
}

#[cfg(test)]
mod tests {
    use crossbeam::channel::{Receiver, unbounded};

    use super::*;

    /// A runner that only records which stages ran, optionally aborting at one
    /// of them.
    struct SpyRunner {
        seen: Arc<Mutex<Vec<Stage>>>,
        abort_at: Option<Stage>,
    }

    impl SpyRunner {
        fn new(abort_at: Option<Stage>) -> (Self, Arc<Mutex<Vec<Stage>>>) {
            let seen = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    seen: Arc::clone(&seen),
                    abort_at,
                },
                seen,
            )
        }

        fn record(&self, stage: Stage) -> Flow {
            self.seen.lock().unwrap().push(stage);
            if self.abort_at == Some(stage) {
                Flow::Abort
            } else {
                Flow::Continue
            }
        }
    }

    impl Runner for SpyRunner {
        fn build(&mut self, _ctx: &RunContext) -> Flow {
            self.record(Stage::Build)
        }

        fn assemble(&mut self, _ctx: &RunContext) -> Flow {
            self.record(Stage::Assemble)
        }

        fn launch(&mut self, _ctx: &RunContext) -> Flow {
            self.record(Stage::Launch)
        }
    }

    /// A runner that implements nothing but the mandatory launch stage, to pin
    /// the defaults down.
    struct LaunchOnlyRunner;

    impl Runner for LaunchOnlyRunner {
        fn launch(&mut self, _ctx: &RunContext) -> Flow {
            Flow::Continue
        }
    }

    /// A context with no child process, so [`drive`] returns as soon as the
    /// stages are done.
    fn context() -> (RunContext, Receiver<RunnerEvent>) {
        let (tx, rx) = unbounded();
        let ctx = RunContext {
            device: Device {
                name: "test".to_string(),
                target: Targets::Macos,
                id: "test".to_string(),
            },
            pkg_name: "my_app".to_string(),
            tx,
            current_child: Arc::new(Mutex::new(None)),
            inspector_address: "127.0.0.1".parse().unwrap(),
            inspector_port: 0,
            release: false,
            session: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        (ctx, rx)
    }

    /// Every status the pipeline reported, in order.
    fn statuses(rx: &Receiver<RunnerEvent>) -> Vec<Status> {
        rx.try_iter()
            .filter_map(|event| match event {
                RunnerEvent::StatusChange(status) => Some(status),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn drive_runs_the_stages_in_pipeline_order() {
        let (runner, seen) = SpyRunner::new(None);
        let (ctx, _rx) = context();

        drive(Box::new(runner), ctx);

        assert_eq!(*seen.lock().unwrap(), Stage::ALL.to_vec());
    }

    #[test]
    fn drive_reports_running_then_idling_after_a_successful_launch() {
        let (runner, _seen) = SpyRunner::new(None);
        let (ctx, rx) = context();

        drive(Box::new(runner), ctx);

        assert_eq!(statuses(&rx), vec![Status::Running, Status::Idling]);
    }

    #[cfg(unix)]
    fn process_exists(pid: u32) -> bool {
        // SAFETY: signal 0 only checks that the pid can be signalled.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    #[cfg(unix)]
    fn wait_until_gone(pid: u32, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if !process_exists(pid) {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        false
    }

    #[cfg(unix)]
    #[test]
    fn terminate_stops_a_child_with_sigterm_and_reaps_it() {
        let child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();

        terminate(child);

        // Gone entirely, not left as a zombie: the helper thread reaps it.
        assert!(wait_until_gone(pid, Duration::from_secs(2)));
    }

    #[cfg(unix)]
    #[test]
    fn terminate_falls_back_to_sigkill_for_a_child_that_ignores_sigterm() {
        let child = std::process::Command::new("sh")
            .args(["-c", "trap '' TERM; while :; do sleep 1; done"])
            .spawn()
            .unwrap();
        let pid = child.id();
        thread::sleep(Duration::from_millis(300));

        terminate(child);

        assert!(process_exists(pid), "SIGTERM alone must not end this child");
        assert!(wait_until_gone(pid, TERMINATE_GRACE + Duration::from_secs(3)));
    }

    #[test]
    fn drive_does_nothing_for_a_run_that_was_already_cancelled() {
        let (runner, seen) = SpyRunner::new(None);
        let (ctx, rx) = context();
        ctx.cancelled.store(true, Ordering::SeqCst);

        drive(Box::new(runner), ctx);

        assert!(seen.lock().unwrap().is_empty());
        assert!(statuses(&rx).is_empty());
    }

    #[test]
    fn stopping_a_runner_handle_cancels_the_run() {
        let child = Arc::new(Mutex::new(None));
        let cancelled = Arc::new(AtomicBool::new(false));
        let handle = RunnerHandle::new(Arc::clone(&child), Arc::clone(&cancelled));

        handle.stop();

        assert!(cancelled.load(Ordering::SeqCst));
        assert!(child.lock().unwrap().is_none());
    }

    #[test]
    fn drive_stops_at_a_failing_build() {
        let (runner, seen) = SpyRunner::new(Some(Stage::Build));
        let (ctx, rx) = context();

        drive(Box::new(runner), ctx);

        assert_eq!(*seen.lock().unwrap(), vec![Stage::Build]);
        assert!(statuses(&rx).is_empty());
    }

    #[test]
    fn drive_stops_at_a_failing_assemble() {
        let (runner, seen) = SpyRunner::new(Some(Stage::Assemble));
        let (ctx, rx) = context();

        drive(Box::new(runner), ctx);

        assert_eq!(*seen.lock().unwrap(), vec![Stage::Build, Stage::Assemble]);
        assert!(statuses(&rx).is_empty());
    }

    #[test]
    fn drive_does_not_report_running_when_the_launch_fails() {
        let (runner, seen) = SpyRunner::new(Some(Stage::Launch));
        let (ctx, rx) = context();

        drive(Box::new(runner), ctx);

        assert_eq!(*seen.lock().unwrap(), Stage::ALL.to_vec());
        assert!(statuses(&rx).is_empty());
    }

    #[test]
    fn build_and_assemble_default_to_doing_nothing() {
        let (ctx, rx) = context();

        drive(Box::new(LaunchOnlyRunner), ctx);

        assert_eq!(statuses(&rx), vec![Status::Running, Status::Idling]);
    }

    #[test]
    fn runner_for_covers_every_runnable_target() {
        for target in [
            Targets::Macos,
            Targets::Web,
            Targets::Ios,
            Targets::IosSimulator,
            Targets::Android,
            Targets::AndroidSimulator,
            Targets::Windows,
            Targets::Linux,
        ] {
            assert!(runner_for(target).is_some(), "{target}");
        }
    }

    #[test]
    fn runner_for_rejects_targets_that_cannot_be_run() {
        assert!(runner_for(Targets::Terminated).is_none());
    }
}
