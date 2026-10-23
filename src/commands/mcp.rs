mod cargo_tools;
mod edit_tools;
mod ide_tools;
mod implementation_results;
mod navigation_tools;
mod position;
mod project_tools;
mod rust_response;
mod session_helpers;
mod symbol_metadata;
mod tool_result;

use position::*;
use rust_response::*;
use session_helpers::*;
use symbol_metadata::symbol_metadata;
use tool_result::*;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow};
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServiceExt, schemars, tool, tool_handler, tool_router, transport::stdio};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::mcp_cargo::{
    CargoRunArguments, CargoRunToolArguments, ManifestEditArguments, ManifestReadArguments,
    ProjectMetadataDetail, prepare_manifest_edit, read_manifest, read_project_metadata_with_detail,
    run_cargo, run_cargo_binary, run_clippy,
};
use super::mcp_docs::{DocsRsClient, GetCrateDocsArguments, SearchCrateDocsArguments};
use super::mcp_ra::{
    InspectFilePattern, InspectFileRange, McpProgressReporter, RustAnalyzerBridge,
    SourceFileChange, compact_document_symbols, inspect_file, normalize_bounded, normalize_result,
    search_text,
};
use super::sessions::resolve_project_path;
use crate::resource_monitor::ResourceRecordStore;
use crate::session::{
    ControlClient, SessionRegistry, SessionSnapshot, SessionStatus,
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
    /// Stable recording ID returned by list_resource_recordings.
    recording_id: String,
    /// Return samples starting at this zero-based offset.
    #[serde(default)]
    cursor: Option<u64>,
    /// Maximum samples to return (1–2000).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustSearchSymbolsArguments {
    /// Fuzzy symbol name or path. Exact case-insensitive name/path matches are ranked first.
    query: String,
    /// Optional Rust symbol kind, such as function, struct, enum, trait, or module.
    #[serde(default)]
    kind: Option<String>,
    /// Maximum results (1–200; defaults to 50).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustTextSearchArguments {
    /// Text or substring to find in project files.
    query: String,
    /// Optional simple glob to restrict paths, for example `src/**/*.rs`.
    #[serde(default)]
    file_glob: Option<String>,
    /// Whether matching should be case-sensitive.
    #[serde(default)]
    case_sensitive: bool,
    /// Interpret query as a regular expression instead of literal text.
    #[serde(default)]
    regex: bool,
    /// Maximum results (1–200; defaults to 50).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustInspectFileArguments {
    /// Project-relative file path.
    file: String,
    /// 1-based inclusive source ranges to return.
    #[serde(default)]
    ranges: Vec<RustInspectFileRangeArguments>,
    /// Literal text or regex searches to run in the same file.
    #[serde(default)]
    searches: Vec<RustInspectFileSearchArguments>,
    /// Output format: `json` (default) or numbered plain `text`.
    #[serde(default)]
    format: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustInspectFileRangeArguments {
    /// First source line, 1-based and inclusive.
    start_line: u32,
    /// Last source line, 1-based and inclusive.
    end_line: u32,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustInspectFileSearchArguments {
    /// Literal text or a Rust regex pattern.
    pattern: String,
    /// Interpret the pattern as a regular expression.
    #[serde(default)]
    regex: bool,
    /// Whether matching should be case-sensitive. Defaults to true.
    #[serde(default)]
    case_sensitive: Option<bool>,
    /// Number of preceding lines to include for each match (0–5).
    #[serde(default)]
    #[schemars(range(max = 5))]
    context_before: Option<usize>,
    /// Number of following lines to include for each match (0–5).
    #[serde(default)]
    #[schemars(range(max = 5))]
    context_after: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustFileArguments {
    /// Project-relative Rust source path.
    file: String,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct RustDocumentSymbolsArguments {
    /// Project-relative Rust source path.
    file: String,
    /// Maximum nested symbol depth (0–8; defaults to 2).
    #[serde(default)]
    #[schemars(range(max = 8))]
    max_depth: Option<usize>,
    /// Keep only these Rust symbol kinds, plus matching ancestors for context.
    #[serde(default)]
    kinds: Vec<String>,
    /// Case-insensitive substring matched against symbol names and details.
    #[serde(default)]
    name_filter: Option<String>,
    /// Include local variable symbols, which are omitted by default.
    #[serde(default)]
    include_variables: bool,
    /// Include field symbols, which are omitted by default.
    #[serde(default)]
    include_fields: bool,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustPositionArguments {
    /// Project-relative Rust source path.
    file: String,
    /// 1-based source line.
    line: u32,
    /// 1-based UTF-16 code-unit column, matching LSP positions.
    column: u32,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct RustImplementationsArguments {
    /// Project-relative Rust source path. Use with line and column instead of symbol_path.
    #[serde(default)]
    file: Option<String>,
    /// 1-based source line for position-based lookup.
    #[serde(default)]
    line: Option<u32>,
    /// 1-based UTF-16 code-unit column for position-based lookup.
    #[serde(default)]
    column: Option<u32>,
    /// Rust symbol path such as `aimer_widget::Drawable`; alternative to file/line/column.
    #[serde(default)]
    symbol_path: Option<String>,
    /// Maximum results per page (1–200; defaults to 50).
    #[serde(default)]
    #[schemars(range(min = 1, max = 200))]
    limit: Option<usize>,
    /// Zero-based result offset.
    #[serde(default)]
    #[schemars(range(max = 1000000))]
    offset: Option<usize>,
    /// Optional simple glob to restrict result paths, for example `crates/*/src/**/*.rs`.
    #[serde(default)]
    file_glob: Option<String>,
    /// Include implementations inside `#[cfg(test)]` and test directories. Defaults to false.
    #[serde(default)]
    include_tests: bool,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustCompletionArguments {
    /// Project-relative Rust source path.
    file: String,
    /// 1-based source line.
    line: u32,
    /// 1-based UTF-16 code-unit column, matching LSP positions.
    column: u32,
    /// Maximum suggestions (1–200; defaults to 50).
    #[serde(default)]
    #[schemars(range(min = 1, max = 200))]
    limit: Option<usize>,
    /// Include shortened documentation for suggestions. Defaults to false.
    #[serde(default)]
    include_documentation: bool,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct RustProjectMetadataArguments {
    /// `summary` by default; use `full` for all bounded Cargo package and resolution metadata.
    #[serde(default)]
    detail: ProjectMetadataDetail,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustDiagnosticsArguments {
    /// Optional project-relative Rust source path. Omit to return diagnostics for all files in the selected project.
    #[serde(default)]
    file: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RustStartLspArguments {
    /// Optional absolute path, or path relative to the currently selected Rust project.
    #[serde(default)]
    project_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustCallHierarchyArguments {
    /// Project-relative Rust source path.
    file: String,
    /// 1-based source line.
    line: u32,
    /// 1-based UTF-16 code-unit column, matching LSP positions.
    column: u32,
    /// `incoming` for callers or `outgoing` for callees.
    direction: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustRangeArguments {
    /// Project-relative Rust source path.
    file: String,
    /// 1-based range start line and UTF-16 code-unit column.
    start_line: u32,
    start_column: u32,
    /// 1-based range end line and UTF-16 code-unit column.
    end_line: u32,
    end_column: u32,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustFormatRangeArguments {
    /// 1-based range start line and UTF-16 code-unit column.
    start_line: u32,
    start_column: u32,
    /// 1-based range end line and UTF-16 code-unit column.
    end_line: u32,
    end_column: u32,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustFormatArguments {
    /// Project-relative Rust source path.
    file: String,
    /// Optional range to format. Omit to format the whole file.
    #[serde(default)]
    range: Option<RustFormatRangeArguments>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustRenameArguments {
    /// Project-relative Rust source path.
    file: String,
    /// 1-based source line and column of the symbol.
    line: u32,
    column: u32,
    /// New Rust identifier name.
    new_name: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustCodeActionPreviewArguments {
    /// Opaque action_id returned by get_code_actions.
    action_id: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustApplyEditArguments {
    /// Opaque preview_id returned by a Rust code, source-edit, or manifest preview tool.
    preview_id: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RustPreviewEditArguments {
    /// File changes to preview. Edit anchors must match exactly once in the original file.
    changes: Vec<SourceFileChange>,
}

#[derive(Clone)]
struct AimerMcpServer {
    client: Option<ControlClient>,
    docs: DocsRsClient,
    project_root: PathBuf,
    rust_analyzer: RustAnalyzerBridge,
    resource_capture_lock: Arc<tokio::sync::Mutex<()>>,
}

impl AimerMcpServer {
    fn tool_router() -> ToolRouter<Self> {
        Self::project_tool_router()
            + Self::ide_tool_router()
            + Self::cargo_tool_router()
            + Self::navigation_tool_router()
            + Self::edit_tool_router()
    }
}

#[tool_handler(
    router = Self::tool_router(),
    instructions = "LSP calls may report indexing progress; wait for the result. Readiness failures are explicit."
)]
impl rmcp::ServerHandler for AimerMcpServer {}

impl AimerMcpServer {
    async fn rust_position_request(
        &self,
        method: &str,
        arguments: RustPositionArguments,
        progress: &McpProgressReporter,
    ) -> CallToolResult {
        encode(
            method,
            self.rust_position_request_value(method, arguments, Value::Null, progress)
                .await,
        )
    }

    async fn rust_position_request_value(
        &self,
        method: &str,
        arguments: RustPositionArguments,
        extra: Value,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<Value> {
        let (uri, source) = self
            .rust_analyzer
            .sync_document(Path::new(&arguments.file), progress)
            .await?;
        let position = lsp_position(&source, arguments.line, arguments.column)?;
        let mut params = json!({
            "textDocument": { "uri": uri },
            "position": position
        });
        if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
            params.extend(extra.clone());
        }
        let result = self.rust_analyzer.request(method, params, progress).await?;
        Ok(normalize_bounded(
            &self.rust_analyzer.project_root(),
            &result,
        ))
    }

    fn request_live(&self, operation: &str, payload: Value) -> anyhow::Result<Value> {
        self.client
            .as_ref()
            .ok_or_else(|| {
                anyhow!("no live Aimer session is attached; use --attach with a session ID for live controls")
            })?
            .request(operation, payload)
    }

    fn require_aimer_project(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            crate::config::AimerManifest::load_from(&self.project_root)?.is_some(),
            "this tool requires an Aimer project containing {}",
            crate::config::MANIFEST_FILE
        );
        Ok(())
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

/// Run the MCP stdio adapter for a selected Aimer session.
pub fn execute(project: Option<PathBuf>, attach: Option<String>) -> anyhow::Result<()> {
    if project.is_some() && attach.is_some() {
        anyhow::bail!("--project and --attach are mutually exclusive");
    }
    let (client, project_root) = select_context(project, attach)?;
    let docs = DocsRsClient::new(project_root.clone())?;
    let rust_analyzer =
        RustAnalyzerBridge::new(project_root.clone(), crate::config::rust_analyzer_config());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting MCP stdio runtime")?;
    runtime.block_on(async move {
        let server = AimerMcpServer {
            client,
            docs,
            project_root,
            rust_analyzer,
            resource_capture_lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        let service = server.serve(stdio()).await?;
        service.waiting().await?;
        Ok::<(), anyhow::Error>(())
    })
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

/// Discover the canonical Aimer or Rust project root by walking upward for
/// `Aimer.toml` or `Cargo.toml`.
pub(crate) fn discover_project_root(start: &Path) -> anyhow::Result<PathBuf> {
    crate::commands::mcp_install::discover_project_root(start)
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
mod tests;
