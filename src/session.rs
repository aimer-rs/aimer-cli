//! Run-session identity, discovery, and the private control protocol used by
//! the aimer mcp adapter.

mod control;
mod runtime;

use control::*;
pub use runtime::{ControlClient, RunSession, SessionRegistry, SessionRuntime};

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, TryLockError, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::config::ResourceMonitorConfig;
use crate::resource_monitor::{
    MAX_SAMPLE_INTERVAL_MS, MIN_SAMPLE_INTERVAL_MS, MonitorInputs, MonitorUpdate,
    ResourceMonitorStatus,
};

const SESSION_DIRECTORY: &str = ".aimer/sessions";
const LOG_CAPACITY: usize = 32_768;
const CONTROL_RETRY_COUNT: usize = 3;
const CONTROL_RETRY_DELAY: Duration = Duration::from_millis(40);
const LIVENESS_RETRY_COUNT: usize = 3;
const LIVENESS_RETRY_DELAY: Duration = Duration::from_millis(10);
const LIVENESS_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60 * 10);
/// Number of attempts for a single control request when the underlying
/// loopback connection fails transiently (e.g. accepted but reset, or closed
/// before a response was written). A busy/accept thread under heavy load can
/// occasionally drop or empty a connection; this retries only those
/// transport-level hiccups, never a genuine application-level error reply.
const CONTROL_REQUEST_RETRY_COUNT: usize = 3;
const CONTROL_REQUEST_RETRY_DELAY: Duration = Duration::from_millis(20);

/// Whether a process belongs to the host or to a connected device.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessScope {
    /// A process addressable by host operating-system tooling.
    Host,
    /// A process addressable through a device bridge or platform adapter.
    Device,
}

/// The role played by a process in a run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessRole {
    /// The application being debugged.
    App,
    /// A development server or log/runner process.
    Runner,
    /// A short-lived launcher process.
    Launcher,
}

/// How an agent may use the process identity for visual inspection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualTarget {
    /// A host PID suitable for host computer-use tooling.
    HostProcess,
    /// An app PID that belongs to a connected device.
    DeviceApp,
    /// A web application that must be opened through its URL.
    WebUrl,
    /// No visual target is currently available.
    Unavailable,
}

/// Structured identity for the process associated with the current run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    /// The process ID, when the platform can resolve one.
    pub pid: Option<u32>,
    /// Where the process runs.
    pub scope: ProcessScope,
    /// The process's role in the run.
    pub role: ProcessRole,
    /// Executable path when known.
    pub executable: Option<PathBuf>,
    /// Bundle/application identifier when the platform exposes one.
    pub bundle_id: Option<String>,
    /// Whether this identity can be passed to a visual tool.
    pub visual_target: VisualTarget,
}

impl ProcessIdentity {
    /// Creates a host application identity.
    pub fn host_app(pid: u32, executable: Option<PathBuf>, bundle_id: Option<String>) -> Self {
        Self {
            pid: Some(pid),
            scope: ProcessScope::Host,
            role: ProcessRole::App,
            executable,
            bundle_id,
            visual_target: VisualTarget::HostProcess,
        }
    }

    /// Creates a device application identity.
    pub fn device_app(pid: Option<u32>, bundle_id: Option<String>) -> Self {
        Self {
            pid,
            scope: ProcessScope::Device,
            role: ProcessRole::App,
            executable: None,
            bundle_id,
            visual_target: VisualTarget::DeviceApp,
        }
    }

    /// Creates a web development-server identity. The URL is carried by the
    /// session snapshot alongside this identity.
    pub fn web_runner(pid: Option<u32>, executable: Option<PathBuf>) -> Self {
        Self {
            pid,
            scope: ProcessScope::Host,
            role: ProcessRole::Runner,
            executable,
            bundle_id: None,
            visual_target: VisualTarget::WebUrl,
        }
    }

    /// Creates an identity for a non-host app without a resolvable PID.
    pub fn unavailable(scope: ProcessScope, bundle_id: Option<String>) -> Self {
        Self {
            pid: None,
            scope,
            role: ProcessRole::App,
            executable: None,
            bundle_id,
            visual_target: VisualTarget::Unavailable,
        }
    }

    /// Returns whether this identity is suitable for host visual tooling.
    #[inline]
    pub fn is_host_visual_process(&self) -> bool {
        self.scope == ProcessScope::Host
            && self.role == ProcessRole::App
            && self.pid.is_some()
            && self.visual_target == VisualTarget::HostProcess
    }
}

/// The lifecycle state reported by a run session.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Starting,
    Locking,
    Fetching,
    Compiling,
    Building,
    Launching,
    Running,
    Idling,
    Restarting,
    Stopping,
    Stopped,
    Error,
}

impl Default for SessionStatus {
    fn default() -> Self {
        Self::Starting
    }
}

/// The configuration that can be replayed by a restart request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RunConfiguration {
    pub project_root: PathBuf,
    pub target: String,
    pub device: Option<String>,
    pub execution_policy: String,
    pub parent_pid: u32,
}

/// A single bounded log record retained by a run session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub seq: u64,
    pub run_id: u64,
    pub stream: LogStream,
    pub level: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

/// Logical source stream for a log query.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    All,
    Build,
    App,
    System,
}

impl LogStream {
    fn matches(self, other: Self) -> bool {
        self == Self::All || self == other
    }
}

/// Query parameters accepted by the log control operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetLogsRequest {
    #[serde(default)]
    pub stream: Option<LogStream>,
    /// Return entries strictly after this sequence number.
    #[serde(default)]
    pub cursor: Option<u64>,
    #[serde(default)]
    pub run_id: Option<u64>,
    #[serde(default = "default_log_limit")]
    pub limit: usize,
    #[serde(default = "default_true")]
    pub include_source_locations: bool,
}

impl Default for GetLogsRequest {
    fn default() -> Self {
        Self {
            stream: None,
            cursor: None,
            run_id: None,
            limit: default_log_limit(),
            include_source_locations: true,
        }
    }
}

fn default_log_limit() -> usize {
    200
}

fn default_true() -> bool {
    true
}

/// A page of logs and the cursor to use for the next page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LogsPage {
    pub entries: Vec<LogEntry>,
    pub next_cursor: Option<u64>,
    pub oldest_seq: Option<u64>,
    pub newest_seq: Option<u64>,
}

/// Result returned after clearing logs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClearLogsResult {
    pub cleared_through: u64,
}

/// Current externally visible session state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub session_id: Uuid,
    pub run_id: u64,
    pub project_root: PathBuf,
    pub target: String,
    pub device: Option<String>,
    pub execution_policy: String,
    pub parent_pid: u32,
    pub status: SessionStatus,
    pub last_error: Option<String>,
    pub app_process: Option<ProcessIdentity>,
    /// Web targets expose the dev-server URL independently of the runner PID.
    pub app_url: Option<String>,
    /// Latest resource sample and active recording status.
    #[serde(default)]
    pub resource_monitor: ResourceMonitorStatus,
}

/// A persisted session descriptor used for discovery.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SessionDescriptor {
    pub session_id: Uuid,
    pub project_root: PathBuf,
    pub endpoint: String,
    pub target: String,
    pub device: Option<String>,
    pub parent_pid: u32,
    pub run_id: u64,
    pub status: SessionStatus,
    pub app_process: Option<ProcessIdentity>,
    pub app_url: Option<String>,
    /// The token is intentionally omitted from sessions output.
    #[serde(skip_serializing)]
    token: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredSessionDescriptor {
    session_id: Uuid,
    project_root: PathBuf,
    endpoint: String,
    token: String,
    target: String,
    device: Option<String>,
    parent_pid: u32,
    run_id: u64,
    status: SessionStatus,
    app_process: Option<ProcessIdentity>,
    app_url: Option<String>,
}

impl StoredSessionDescriptor {
    fn into_descriptor(self) -> SessionDescriptor {
        SessionDescriptor {
            session_id: self.session_id,
            project_root: self.project_root,
            endpoint: self.endpoint,
            target: self.target,
            device: self.device,
            parent_pid: self.parent_pid,
            run_id: self.run_id,
            status: self.status,
            app_process: self.app_process,
            app_url: self.app_url,
            token: self.token,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct WireResponse {
    request_id: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct WireRequest {
    request_id: String,
    token: String,
    operation: String,
    #[serde(default)]
    payload: Value,
}

/// A command delivered from the control endpoint to the run loop.
pub enum SessionCommand {
    /// Stop the current run and leave the session stopped.
    Stop {
        reply: mpsc::Sender<Result<SessionSnapshot, String>>,
    },
    /// Restart the run with the original configuration.
    Restart {
        reply: mpsc::Sender<Result<SessionSnapshot, String>>,
    },
    /// Clear retained logs.
    ClearLogs {
        request: GetLogsRequest,
        reply: mpsc::Sender<Result<ClearLogsResult, String>>,
    },
}

enum PendingMutation {
    Restart(mpsc::Sender<Result<SessionSnapshot, String>>),
}

struct SessionState {
    run_id: u64,
    status: SessionStatus,
    last_error: Option<String>,
    app_process: Option<ProcessIdentity>,
    app_url: Option<String>,
    resource_monitor: ResourceMonitorStatus,
    next_seq: u64,
    logs: VecDeque<LogEntry>,
    commands: VecDeque<SessionCommand>,
    pending_mutation: Option<PendingMutation>,
}

struct SharedSession {
    session_id: Uuid,
    configuration: RunConfiguration,
    resource_config: ResourceMonitorConfig,
    control_enabled: bool,
    endpoint: Mutex<String>,
    token: String,
    descriptor_path: PathBuf,
    descriptor_write: Mutex<()>,
    state: Mutex<SessionState>,
    wake: Condvar,
    mutation: Mutex<()>,
}

/// A cloneable handle used by the run loop to publish state and consume
/// control commands.
#[derive(Clone)]
pub struct SessionHandle {
    shared: Arc<SharedSession>,
    run_id: Option<u64>,
}

impl SessionHandle {
    /// Returns the stable session identifier.
    #[inline]
    pub fn session_id(&self) -> Uuid {
        self.shared.session_id
    }

    /// Returns a snapshot suitable for status output.
    pub fn snapshot(&self) -> SessionSnapshot {
        let state = self.shared.state.lock().expect("session mutex poisoned");
        snapshot_locked(&self.shared, &state)
    }

    pub(crate) fn monitor_inputs(&self) -> MonitorInputs {
        let state = self.shared.state.lock().expect("session mutex poisoned");
        MonitorInputs {
            session_id: self.shared.session_id,
            run_id: state.run_id,
            project_root: self.shared.configuration.project_root.clone(),
            target: self.shared.configuration.target.clone(),
            device: self.shared.configuration.device.clone(),
            status: state.status,
            app_process: state.app_process.clone(),
            interval_ms: state.resource_monitor.interval_ms,
            retention_days: self.shared.resource_config.retention_days,
        }
    }

    pub(crate) fn publish_resource_sample(&self, update: MonitorUpdate) {
        let mut state = self.shared.state.lock().expect("session mutex poisoned");
        if update.run_id != state.run_id
            || self.run_id.is_some_and(|run_id| run_id != state.run_id)
            || !matches!(state.status, SessionStatus::Running | SessionStatus::Error)
            || state.app_process.as_ref().and_then(|process| process.pid) != update.sample.pid
        {
            return;
        }
        state.resource_monitor.sample = Some(update.sample);
        state.resource_monitor.recording = update.recording;
    }

    pub(crate) fn clear_resource_sample(&self) {
        let mut state = self.shared.state.lock().expect("session mutex poisoned");
        state.resource_monitor.recording = false;
        state.resource_monitor.sample = None;
    }

    pub(crate) fn wait_for_monitor_wake(&self, duration: Duration) {
        let state = self.shared.state.lock().expect("session mutex poisoned");
        let _ = self
            .shared
            .wake
            .wait_timeout(state, duration)
            .expect("session mutex poisoned while waiting for resource monitor");
    }

    fn set_resource_interval(
        &self,
        interval_ms: Option<u64>,
    ) -> Result<ResourceMonitorStatus, String> {
        let interval_ms = interval_ms.unwrap_or(self.shared.resource_config.sample_interval_ms);
        if !(MIN_SAMPLE_INTERVAL_MS..=MAX_SAMPLE_INTERVAL_MS).contains(&interval_ms) {
            return Err(format!(
                "sample interval must be between {MIN_SAMPLE_INTERVAL_MS} and {MAX_SAMPLE_INTERVAL_MS} milliseconds"
            ));
        }
        let mut state = self.shared.state.lock().expect("session mutex poisoned");
        state.resource_monitor.interval_ms = interval_ms;
        let status = state.resource_monitor.clone();
        drop(state);
        self.shared.wake.notify_all();
        Ok(status)
    }

    /// Creates a handle scoped to the current run. Lifecycle events from a
    /// previous runner then cannot clear the identity of a newly restarted
    /// app.
    pub fn scoped(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            run_id: Some(self.snapshot().run_id),
        }
    }

    /// Publish a lifecycle status. Running, Idling, and Error also complete a
    /// pending restart request.
    pub fn set_status(&self, status: SessionStatus, last_error: Option<String>) {
        let mut pending = None;
        {
            let mut state = self.shared.state.lock().expect("session mutex poisoned");
            if self.run_id.is_some_and(|run_id| run_id != state.run_id) {
                return;
            }
            state.status = status;
            if last_error.is_some() {
                state.last_error = last_error;
            } else if status != SessionStatus::Error {
                state.last_error = None;
            }
            let app_process_survives_error = status == SessionStatus::Error
                && state
                    .app_process
                    .as_ref()
                    .is_some_and(|process| process.role == ProcessRole::App);
            if status != SessionStatus::Running && !app_process_survives_error {
                state.resource_monitor.recording = false;
                state.resource_monitor.sample = None;
            }
            if matches!(status, SessionStatus::Idling | SessionStatus::Stopped) {
                state.app_process = None;
                state.app_url = None;
            }
            if matches!(status, SessionStatus::Running | SessionStatus::Error)
                && let Some(PendingMutation::Restart(reply)) = state.pending_mutation.take()
            {
                pending = Some((reply, status, snapshot_locked(&self.shared, &state)));
            }
        }
        self.persist_descriptor();
        if let Some((reply, status, snapshot)) = pending {
            let result = if status == SessionStatus::Running {
                Ok(snapshot)
            } else {
                Err(snapshot
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "project restart failed".to_string()))
            };
            let _ = reply.send(result);
        }
        self.shared.wake.notify_all();
    }

    /// Set or replace the current application or runner process identity.
    pub fn set_process(&self, process: Option<ProcessIdentity>, app_url: Option<String>) {
        {
            let mut state = self.shared.state.lock().expect("session mutex poisoned");
            if self.run_id.is_some_and(|run_id| run_id != state.run_id) {
                return;
            }
            if process.is_none() {
                state.resource_monitor.recording = false;
                state.resource_monitor.sample = None;
            }
            state.app_process = process;
            state.app_url = app_url;
        }
        self.persist_descriptor();
        self.shared.wake.notify_all();
    }

    /// Clear the current process as soon as a restart or app exit begins.
    pub fn clear_process(&self) {
        self.set_process(None, None);
    }

    /// Advance to a fresh run for a restart initiated by the local console.
    /// MCP initiated restarts use `accept_restart`, which also carries a reply
    /// channel for the requesting client.
    pub fn begin_next_run(&self) -> Result<u64, String> {
        let _mutation = match self.shared.mutation.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return Err("busy".to_string()),
            Err(TryLockError::Poisoned(_)) => {
                return Err("session mutation lock poisoned".to_string());
            }
        };
        let run_id = {
            let mut state = self.shared.state.lock().expect("session mutex poisoned");
            if self.run_id.is_some_and(|run_id| run_id != state.run_id) {
                return Err("stale session handle".to_string());
            }
            if state.pending_mutation.is_some() {
                return Err("busy".to_string());
            }
            state.run_id = state.run_id.saturating_add(1);
            state.status = SessionStatus::Restarting;
            state.last_error = None;
            state.app_process = None;
            state.app_url = None;
            state.resource_monitor.recording = false;
            state.resource_monitor.sample = None;
            state.run_id
        };
        self.persist_descriptor();
        self.shared.wake.notify_all();
        Ok(run_id)
    }

    /// Append a bounded log record to the current run.
    pub fn push_log(
        &self,
        stream: LogStream,
        level: impl Into<String>,
        message: impl Into<String>,
        location: Option<String>,
    ) {
        let mut state = self.shared.state.lock().expect("session mutex poisoned");
        let entry = LogEntry {
            seq: state.next_seq,
            run_id: state.run_id,
            stream,
            level: level.into(),
            message: message.into(),
            location,
        };
        state.next_seq = state.next_seq.saturating_add(1);
        state.logs.push_back(entry);
        while state.logs.len() > LOG_CAPACITY {
            state.logs.pop_front();
        }
    }

    /// Read a cursor-paginated page of logs.
    pub fn get_logs(&self, request: GetLogsRequest) -> LogsPage {
        let state = self.shared.state.lock().expect("session mutex poisoned");
        logs_locked(&state, request)
    }

    /// Take the next mutation command, if one is waiting for the run loop.
    pub fn take_command(&self) -> Option<SessionCommand> {
        self.shared
            .state
            .lock()
            .expect("session mutex poisoned")
            .commands
            .pop_front()
    }

    /// Mark a restart command as accepted. The PID is cleared and run_id
    /// increments before the old child is killed.
    pub fn accept_restart(
        &self,
        reply: mpsc::Sender<Result<SessionSnapshot, String>>,
    ) -> anyhow::Result<()> {
        {
            let mut state = self.shared.state.lock().expect("session mutex poisoned");
            if state.pending_mutation.is_some() {
                anyhow::bail!("busy")
            }
            state.run_id = state.run_id.saturating_add(1);
            state.status = SessionStatus::Restarting;
            state.last_error = None;
            state.app_process = None;
            state.app_url = None;
            state.resource_monitor.recording = false;
            state.resource_monitor.sample = None;
            state.pending_mutation = Some(PendingMutation::Restart(reply));
        }
        self.persist_descriptor();
        self.shared.wake.notify_all();
        Ok(())
    }

    /// Complete a restart immediately when the runner cannot be started.
    pub fn fail_restart(&self, error: impl Into<String>) {
        let reply = {
            let mut state = self.shared.state.lock().expect("session mutex poisoned");
            state.status = SessionStatus::Error;
            state.last_error = Some(error.into());
            state.resource_monitor.recording = false;
            state.resource_monitor.sample = None;
            match state.pending_mutation.take() {
                Some(PendingMutation::Restart(reply)) => Some(reply),
                None => None,
            }
        };
        self.persist_descriptor();
        if let Some(reply) = reply {
            let _ = reply.send(Err(self
                .snapshot()
                .last_error
                .unwrap_or_else(|| "project restart failed".to_string())));
        }
    }

    /// Mark the session stopped and answer a stop request.
    pub fn complete_stop(&self, reply: mpsc::Sender<Result<SessionSnapshot, String>>) {
        self.clear_process();
        self.set_status(SessionStatus::Stopped, None);
        let _ = reply.send(Ok(self.snapshot()));
    }

    /// Clear logs for the requested stream and reply with the sequence
    /// boundary. This is called by the run loop so it shares mutation
    /// serialization with restart and stop.
    pub fn complete_clear_logs(
        &self,
        request: GetLogsRequest,
        reply: mpsc::Sender<Result<ClearLogsResult, String>>,
    ) {
        let boundary = {
            let mut state = self.shared.state.lock().expect("session mutex poisoned");
            let boundary = state.next_seq.saturating_sub(1);
            let stream = request.stream.unwrap_or(LogStream::All);
            if stream == LogStream::All && request.run_id.is_none() {
                state.logs.clear();
            } else {
                state.logs.retain(|entry| {
                    !(stream.matches(entry.stream)
                        && request.run_id.map_or(true, |run_id| entry.run_id == run_id))
                });
            }
            boundary
        };
        let _ = reply.send(Ok(ClearLogsResult {
            cleared_through: boundary,
        }));
    }

    fn enqueue_command(&self, command: SessionCommand) {
        let mut state = self.shared.state.lock().expect("session mutex poisoned");
        state.commands.push_back(command);
        self.shared.wake.notify_all();
    }

    fn request_mutation<T>(
        &self,
        command: impl FnOnce(mpsc::Sender<Result<T, String>>) -> SessionCommand,
    ) -> Result<T, String> {
        let _guard = match self.shared.mutation.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return Err("busy".to_string()),
            Err(TryLockError::Poisoned(_)) => {
                return Err("session mutation lock poisoned".to_string());
            }
        };
        let (reply, receiver) = mpsc::channel();
        self.enqueue_command(command(reply));
        receiver
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|_| "timed out waiting for the run loop".to_string())?
    }

    fn request_status(&self, operation: &str, payload: Value) -> Result<Value, String> {
        match operation {
            "ping" | "status" => serde_json::to_value(self.snapshot()).map_err(|e| e.to_string()),
            "logs" => {
                let request: GetLogsRequest =
                    serde_json::from_value(payload).map_err(|e| e.to_string())?;
                serde_json::to_value(self.get_logs(request)).map_err(|e| e.to_string())
            }
            "resource_usage" => serde_json::to_value(self.snapshot().resource_monitor)
                .map_err(|error| error.to_string()),
            "set_resource_interval" => {
                #[derive(Deserialize)]
                struct IntervalRequest {
                    #[serde(default)]
                    interval_ms: Option<u64>,
                }
                let request: IntervalRequest = if payload.is_null() {
                    IntervalRequest { interval_ms: None }
                } else {
                    serde_json::from_value(payload).map_err(|error| error.to_string())?
                };
                serde_json::to_value(self.set_resource_interval(request.interval_ms)?)
                    .map_err(|error| error.to_string())
            }
            "restart" => {
                let snapshot = self.request_mutation(|reply| SessionCommand::Restart { reply })?;
                serde_json::to_value(snapshot).map_err(|e| e.to_string())
            }
            "stop" => {
                let snapshot = self.request_mutation(|reply| SessionCommand::Stop { reply })?;
                serde_json::to_value(snapshot).map_err(|e| e.to_string())
            }
            "clear_logs" => {
                let request: GetLogsRequest = if payload.is_null() {
                    GetLogsRequest::default()
                } else {
                    serde_json::from_value(payload).map_err(|e| e.to_string())?
                };
                let result =
                    self.request_mutation(|reply| SessionCommand::ClearLogs { request, reply })?;
                serde_json::to_value(result).map_err(|e| e.to_string())
            }
            other => Err(format!("unknown control operation '{other}'")),
        }
    }

    fn persist_descriptor(&self) {
        if let Err(error) = self.persist_descriptor_checked() {
            tracing::debug!(error = %error, "failed to update aimer session descriptor");
        }
    }

    fn persist_descriptor_checked(&self) -> anyhow::Result<()> {
        if !self.shared.control_enabled {
            return Ok(());
        }
        let _write_guard = self
            .shared
            .descriptor_write
            .lock()
            .expect("descriptor mutex poisoned");
        let state = self.shared.state.lock().expect("session mutex poisoned");
        let descriptor = StoredSessionDescriptor {
            session_id: self.shared.session_id,
            project_root: self.shared.configuration.project_root.clone(),
            endpoint: self
                .shared
                .endpoint
                .lock()
                .expect("endpoint mutex poisoned")
                .clone(),
            token: self.shared.token.clone(),
            target: self.shared.configuration.target.clone(),
            device: self.shared.configuration.device.clone(),
            parent_pid: self.shared.configuration.parent_pid,
            run_id: state.run_id,
            status: state.status,
            app_process: state.app_process.clone(),
            app_url: state.app_url.clone(),
        };
        drop(state);
        write_descriptor(&self.shared.descriptor_path, &descriptor)
    }
}

/// The server registration kept alive for the duration of an aimer run.
fn sessions_directory() -> anyhow::Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(SESSION_DIRECTORY))
        .ok_or_else(|| anyhow!("could not determine the user home directory"))
}

fn normalize_project_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|current| current.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    })
}

fn read_descriptor(path: &Path) -> anyhow::Result<SessionDescriptor> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("reading session descriptor {}", path.display()))?;
    Ok(serde_json::from_str::<StoredSessionDescriptor>(&contents)?.into_descriptor())
}

fn write_descriptor(path: &Path, descriptor: &StoredSessionDescriptor) -> anyhow::Result<()> {
    let temporary = path.with_extension("json.tmp");
    let contents = serde_json::to_vec_pretty(descriptor)?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(&contents)?;
    file.write_all(b"\n")?;
    file.sync_data().ok();
    drop(file);
    fs::rename(&temporary, path)?;
    set_descriptor_permissions(path);
    Ok(())
}

#[cfg(unix)]
fn set_descriptor_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[cfg(not(unix))]
fn set_descriptor_permissions(_path: &Path) {}

#[cfg(test)]
mod tests;
