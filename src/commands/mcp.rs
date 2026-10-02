use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ServiceExt, schemars, tool, tool_router, transport::stdio};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::mcp_docs::{DocsRsClient, GetCrateDocsArguments, SearchCrateDocsArguments};
use crate::resource_monitor::ResourceRecordStore;
use crate::session::{
    ControlClient, SessionDescriptor, SessionRegistry, SessionSnapshot, SessionStatus,
};

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct LogsArguments {
    /// One of all, build, app, or system.
    #[serde(default)]
    stream: Option<String>,
    /// Return entries strictly after this sequence number.
    #[serde(default)]
    cursor: Option<u64>,
    /// Restrict results to one run ID.
    #[serde(default)]
    run_id: Option<u64>,
    /// Maximum number of entries to return.
    #[serde(default)]
    limit: Option<usize>,
    /// Include source file and line information when available.
    #[serde(default)]
    include_source_locations: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct ResourceIntervalArguments {
    /// Requested sampling interval in milliseconds (500–3000). Omit to restore
    /// the global Aimer configuration value, which defaults to 1000 ms.
    #[serde(default)]
    interval_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct ResourceCaptureArguments {
    /// Capture duration in seconds (0.5–300), and at least as long as interval.
    #[schemars(range(min = 0.5, max = 300.0))]
    duration: f64,
    /// Sampling interval in seconds (0.5–3.0).
    #[schemars(range(min = 0.5, max = 3.0))]
    interval: f64,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct ResourceRecordingsArguments {
    /// Return entries after this list offset.
    #[serde(default)]
    cursor: Option<u64>,
    /// Maximum recordings to return (1–100).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct ResourceRecordingArguments {
    /// Stable recording ID returned by aimer_list_resource_recordings.
    recording_id: String,
    /// Return samples starting at this zero-based offset.
    #[serde(default)]
    cursor: Option<u64>,
    /// Maximum samples to return (1–2000).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Clone)]
struct AimerMcpServer {
    client: Option<ControlClient>,
    docs: DocsRsClient,
    project_root: PathBuf,
    resource_capture_lock: Arc<tokio::sync::Mutex<()>>,
}

#[tool_router(server_handler)]
impl AimerMcpServer {
    #[tool(
        name = "aimer_status",
        description = "Return the current Aimer run status and app process identity."
    )]
    fn status(&self) -> String {
        encode(self.request_live("status", Value::Null))
    }

    #[tool(
        name = "aimer_restart_project",
        description = "Restart the attached Aimer project and wait for the new app run to start."
    )]
    fn restart_project(&self) -> String {
        encode(self.request_live("restart", Value::Null))
    }

    #[tool(
        name = "aimer_stop_project",
        description = "Stop the attached Aimer project."
    )]
    fn stop_project(&self) -> String {
        encode(self.request_live("stop", Value::Null))
    }

    #[tool(
        name = "aimer_get_logs",
        description = "Read bounded Aimer build, app, or system logs with cursor pagination."
    )]
    fn get_logs(&self, Parameters(arguments): Parameters<LogsArguments>) -> String {
        encode(self.request_live("logs", logs_payload(arguments)))
    }

    #[tool(
        name = "aimer_clear_logs",
        description = "Clear retained Aimer logs and return the sequence boundary that was cleared."
    )]
    fn clear_logs(&self, Parameters(arguments): Parameters<LogsArguments>) -> String {
        encode(self.request_live("clear_logs", logs_payload(arguments)))
    }

    #[tool(
        name = "aimer_get_resource_usage",
        description = "Return the latest automatic CPU, memory, PID, thread, and GPU resource sample for the attached running Aimer app."
    )]
    fn get_resource_usage(&self) -> String {
        encode(self.request_live("resource_usage", Value::Null))
    }

    #[tool(
        name = "aimer_set_resource_monitor_interval",
        description = "Set the attached app's live resource sampling interval to 500–3000 ms. Omit interval_ms to restore the global Aimer configuration default (1000 ms)."
    )]
    async fn set_resource_monitor_interval(
        &self,
        Parameters(arguments): Parameters<ResourceIntervalArguments>,
    ) -> String {
        let _capture_lock = self.resource_capture_lock.lock().await;
        let payload = arguments
            .interval_ms
            .map(|interval_ms| json!({ "interval_ms": interval_ms }))
            .unwrap_or_else(|| json!({}));
        encode(self.request_live("set_resource_interval", payload))
    }

    #[tool(
        name = "aimer_record_resource_usage",
        description = "Record a bounded window of the attached app's resource usage. Takes duration in seconds (0.5–300) and interval in seconds (0.5–3), temporarily changes the live sampling interval, restores it afterward, and returns samples plus the run recording ID."
    )]
    async fn record_resource_usage(
        &self,
        Parameters(arguments): Parameters<ResourceCaptureArguments>,
    ) -> String {
        let result = self.capture_resource_usage(arguments).await;
        encode(result)
    }

    #[tool(
        name = "aimer_list_resource_recordings",
        description = "List retained per-run resource recordings for the attached Aimer project, including completed runs."
    )]
    fn list_resource_recordings(
        &self,
        Parameters(arguments): Parameters<ResourceRecordingsArguments>,
    ) -> String {
        let result = ResourceRecordStore::for_project(&self.project_root).and_then(|store| {
            let retention_days = crate::config::resource_monitor_config().retention_days;
            let page = store.list(retention_days, arguments.cursor, arguments.limit)?;
            serde_json::to_value(page).context("encoding resource recording list")
        });
        encode(result)
    }

    #[tool(
        name = "aimer_get_resource_recording",
        description = "Read cursor-paginated timestamped samples from a retained per-run resource recording."
    )]
    fn get_resource_recording(
        &self,
        Parameters(arguments): Parameters<ResourceRecordingArguments>,
    ) -> String {
        let result = ResourceRecordStore::for_project(&self.project_root).and_then(|store| {
            let retention_days = crate::config::resource_monitor_config().retention_days;
            let page = store.read(
                &arguments.recording_id,
                arguments.cursor,
                arguments.limit,
                retention_days,
            )?;
            serde_json::to_value(page).context("encoding resource recording samples")
        });
        encode(result)
    }

    #[tool(
        name = "aimer_get_crate_docs",
        description = "Read docs.rs documentation for a Rust crate or item. Trait lookups also return the trait signature, all method signatures, required/provided status, and each method's documentation. Uses the attached project's Cargo.lock version by default and caches crate documentation for offline use."
    )]
    async fn get_crate_docs(
        &self,
        Parameters(arguments): Parameters<GetCrateDocsArguments>,
    ) -> String {
        let result = self
            .docs
            .get_document(arguments)
            .await
            .and_then(|document| serde_json::to_value(document).context("encoding docs.rs result"));
        encode(result)
    }

    #[tool(
        name = "aimer_search_crate_docs",
        description = "Search public Rust item names, paths, and documentation in a docs.rs crate. Uses the attached project's Cargo.lock version by default, returns bounded results with item paths and summaries, and shares the offline documentation cache with aimer_get_crate_docs."
    )]
    async fn search_crate_docs(
        &self,
        Parameters(arguments): Parameters<SearchCrateDocsArguments>,
    ) -> String {
        let result = self
            .docs
            .search_documents(arguments)
            .await
            .and_then(|results| {
                serde_json::to_value(results).context("encoding docs.rs search results")
            });
        encode(result)
    }
}

impl AimerMcpServer {
    fn request_live(&self, operation: &str, payload: Value) -> anyhow::Result<Value> {
        self.client
            .as_ref()
            .ok_or_else(|| {
                anyhow!("no live Aimer session is attached; use --attach with a session ID for live controls")
            })?
            .request(operation, payload)
    }

    async fn capture_resource_usage(
        &self,
        arguments: ResourceCaptureArguments,
    ) -> anyhow::Result<Value> {
        anyhow::ensure!(
            arguments.interval.is_finite() && (0.5..=3.0).contains(&arguments.interval),
            "interval must be between 0.5 and 3 seconds"
        );
        anyhow::ensure!(
            arguments.duration.is_finite()
                && arguments.duration >= arguments.interval
                && arguments.duration <= 300.0,
            "duration must be at least interval and no more than 300 seconds"
        );

        let _capture_lock = self.resource_capture_lock.lock().await;
        let snapshot: SessionSnapshot =
            serde_json::from_value(self.request_live("ping", Value::Null)?)
                .context("decoding live Aimer session status")?;
        anyhow::ensure!(
            matches!(
                snapshot.status,
                SessionStatus::Running | SessionStatus::Error
            ) && snapshot.app_process.is_some(),
            "the attached Aimer app is not running"
        );

        let interval_ms = (arguments.interval * 1_000.0).round() as u64;
        let client = self
            .client
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("no live Aimer session is attached"))?;
        let mut interval_restore =
            ResourceIntervalRestore::new(client, snapshot.resource_monitor.interval_ms);
        interval_restore.set_interval(interval_ms)?;

        let capture_started_unix_ms = now_unix_ms()?;
        let capture_started = tokio::time::Instant::now();
        tokio::time::sleep(Duration::from_secs_f64(arguments.duration)).await;
        let capture_finished_unix_ms = now_unix_ms()?;
        let captured_duration_seconds = capture_started.elapsed().as_secs_f64();
        let interval_restore_error = interval_restore
            .restore()
            .err()
            .map(|error| error.to_string());

        let retention_days = crate::config::resource_monitor_config().retention_days;
        let store = ResourceRecordStore::for_project(&self.project_root)?;
        let recording_id = format!("{}_{}", snapshot.session_id, snapshot.run_id);
        let metadata = store
            .list(retention_days, None, Some(100))?
            .recordings
            .into_iter()
            .find(|recording| recording.recording_id == recording_id)
            .ok_or_else(|| anyhow!("resource recording '{recording_id}' was not found"))?;
        let expected_samples = (arguments.duration / arguments.interval).ceil() as u64 + 8;
        let cursor = metadata.sample_count.saturating_sub(expected_samples);
        let page = store.read(&recording_id, Some(cursor), Some(2_000), retention_days)?;
        let samples: Vec<_> = page
            .samples
            .into_iter()
            .filter(|sample| {
                (capture_started_unix_ms..=capture_finished_unix_ms)
                    .contains(&sample.timestamp_unix_ms)
            })
            .collect();

        Ok(json!({
            "recording_id": recording_id,
            "duration_seconds": captured_duration_seconds,
            "interval_seconds": interval_ms as f64 / 1_000.0,
            "capture_started_unix_ms": capture_started_unix_ms,
            "capture_finished_unix_ms": capture_finished_unix_ms,
            "sample_count": samples.len(),
            "samples": samples,
            "interval_restored": interval_restore_error.is_none(),
            "interval_restore_error": interval_restore_error,
        }))
    }
}

struct ResourceIntervalRestore {
    client: ControlClient,
    interval_ms: u64,
    armed: bool,
}

impl ResourceIntervalRestore {
    fn new(client: ControlClient, interval_ms: u64) -> Self {
        Self {
            client,
            interval_ms,
            armed: true,
        }
    }

    fn set_interval(&self, interval_ms: u64) -> anyhow::Result<()> {
        self.client.request(
            "set_resource_interval",
            json!({ "interval_ms": interval_ms }),
        )?;
        Ok(())
    }

    fn restore(&mut self) -> anyhow::Result<()> {
        self.set_interval(self.interval_ms)?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for ResourceIntervalRestore {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error) = self.set_interval(self.interval_ms) {
                tracing::debug!(error = %error, "could not restore resource sampling interval");
            }
        }
    }
}

fn now_unix_ms() -> anyhow::Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_millis() as u64)
}

fn encode(result: anyhow::Result<Value>) -> String {
    match result {
        Ok(value) => serde_json::to_string(&value)
            .unwrap_or_else(|error| json!({ "error": error.to_string() }).to_string()),
        Err(error) => json!({ "error": error.to_string() }).to_string(),
    }
}

fn logs_payload(arguments: LogsArguments) -> Value {
    let mut payload = Map::new();
    if let Some(stream) = arguments.stream.filter(|stream| stream != "all") {
        payload.insert("stream".to_string(), Value::String(stream));
    }
    if let Some(cursor) = arguments.cursor {
        payload.insert("cursor".to_string(), json!(cursor));
    }
    if let Some(run_id) = arguments.run_id {
        payload.insert("run_id".to_string(), json!(run_id));
    }
    if let Some(limit) = arguments.limit {
        payload.insert("limit".to_string(), json!(limit));
    }
    if let Some(include) = arguments.include_source_locations {
        payload.insert("include_source_locations".to_string(), json!(include));
    }
    Value::Object(payload)
}

/// Run the MCP stdio adapter for a selected Aimer session.
pub fn execute(project: Option<PathBuf>, attach: Option<String>) -> anyhow::Result<()> {
    if project.is_some() && attach.is_some() {
        anyhow::bail!("--project and --attach are mutually exclusive");
    }
    let (client, project_root) = select_context(project, attach)?;
    let docs = DocsRsClient::new(project_root.clone())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting MCP stdio runtime")?;
    runtime.block_on(async move {
        let server = AimerMcpServer {
            client,
            docs,
            project_root,
            resource_capture_lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        let service = server.serve(stdio()).await?;
        service.waiting().await?;
        Ok::<(), anyhow::Error>(())
    })
}

/// List live session descriptors for the sessions command.
pub fn list_sessions(project: Option<PathBuf>, json_output: bool) -> anyhow::Result<()> {
    let project = project.map(resolve_project_path).transpose()?;
    let registry = SessionRegistry::default_location()?;
    let sessions = registry.live(project.as_deref())?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }
    if sessions.is_empty() {
        println!("No live Aimer run sessions.");
        return Ok(());
    }
    for session in sessions {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            session.session_id,
            session.status_name(),
            session.target,
            session
                .app_process
                .as_ref()
                .and_then(|process| process.pid)
                .map_or_else(|| "-".to_string(), |pid| pid.to_string()),
            session.project_root.display(),
        );
    }
    Ok(())
}

fn select_context(
    project: Option<PathBuf>,
    attach: Option<String>,
) -> anyhow::Result<(Option<ControlClient>, PathBuf)> {
    let registry = SessionRegistry::default_location()?;
    if let Some(attach) = attach {
        let client = registry.attach(parse_session_id(&attach)?)?;
        return Ok((
            Some(client.clone()),
            client.descriptor().project_root.clone(),
        ));
    }
    let project = match project {
        Some(project) => resolve_project_path(project)?,
        None => discover_project_root(&std::env::current_dir()?)?,
    };
    let sessions = registry.live(Some(&project))?;
    match sessions.as_slice() {
        [] => Ok((None, project)),
        [session] => Ok((Some(registry.attach(session.session_id)?), project)),
        _ => Ok((None, project)),
    }
}

fn resolve_project_path(path: PathBuf) -> anyhow::Result<PathBuf> {
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    path.canonicalize()
        .with_context(|| format!("project path '{}' does not exist", path.display()))
}

/// Discover the canonical Aimer project root by walking upward for
/// `Aimer.toml`.
pub(crate) fn discover_project_root(start: &Path) -> anyhow::Result<PathBuf> {
    crate::commands::mcp_install::discover_project_root(start)
}

impl SessionDescriptor {
    fn status_name(&self) -> &'static str {
        match self.status {
            crate::session::SessionStatus::Starting => "starting",
            crate::session::SessionStatus::Locking => "locking",
            crate::session::SessionStatus::Fetching => "fetching",
            crate::session::SessionStatus::Compiling => "compiling",
            crate::session::SessionStatus::Building => "building",
            crate::session::SessionStatus::Launching => "launching",
            crate::session::SessionStatus::Running => "running",
            crate::session::SessionStatus::Idling => "idling",
            crate::session::SessionStatus::Restarting => "restarting",
            crate::session::SessionStatus::Stopping => "stopping",
            crate::session::SessionStatus::Stopped => "stopped",
            crate::session::SessionStatus::Error => "error",
        }
    }
}

/// Parse an exact session ID for the CLI.
pub fn parse_session_id(value: &str) -> anyhow::Result<Uuid> {
    Uuid::parse_str(value).with_context(|| format!("invalid session ID '{value}'"))
}

/// Attach to an exact session. This helper is kept separate so tests and
/// future non-MCP adapters can use the same identity validation.
pub fn attach_exact(value: &str) -> anyhow::Result<ControlClient> {
    let registry = SessionRegistry::default_location()?;
    registry.attach(parse_session_id(value)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_payload_omits_unspecified_fields() {
        let payload = logs_payload(LogsArguments {
            stream: Some("app".to_string()),
            ..LogsArguments::default()
        });
        assert_eq!(payload["stream"], "app");
        assert!(payload.get("cursor").is_none());
    }

    #[test]
    fn log_payload_treats_all_as_the_default_stream() {
        let payload = logs_payload(LogsArguments {
            stream: Some("all".to_string()),
            limit: Some(5),
            ..LogsArguments::default()
        });

        assert!(payload.get("stream").is_none());
        assert_eq!(payload["limit"], 5);
    }

    #[test]
    fn global_mcp_discovery_uses_the_nearest_aimer_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let nested = project.join("src/deep");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(project.join("Aimer.toml"), "[package]\nname = \"demo\"\n").unwrap();

        assert_eq!(
            discover_project_root(&nested).unwrap(),
            project.canonicalize().unwrap()
        );
    }

    #[test]
    fn global_mcp_discovery_rejects_directories_without_aimer_manifest() {
        let temp = tempfile::tempdir().unwrap();

        let error = discover_project_root(temp.path()).unwrap_err().to_string();

        assert!(error.contains("no Aimer.toml found"));
    }
}
