//! A small rust-analyzer LSP client used by Aimer's MCP adapter.
//!
//! The MCP process owns one rust-analyzer process for its selected project,
//! started on the first Rust request or explicitly by the start tool. The
//! client deliberately exposes structured JSON instead of LSP's raw protocol
//! messages.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use rmcp::model::{ProgressNotificationParam, ProgressToken};
use rmcp::service::RequestContext;
use rmcp::{Peer, RoleServer};
use serde_json::{Map, Value, json};
use syn::visit::Visit;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, mpsc};
use tokio::time::timeout;
use uuid::Uuid;

const MAX_LSP_MESSAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_PENDING_LSP_MESSAGES: usize = 128;
const MAX_RESULTS: usize = 200;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
const INDEXING_TIMEOUT: Duration = Duration::from_secs(300);
const INDEXING_POLL_INTERVAL: Duration = Duration::from_millis(250);
const INDEXING_PROGRESS_INTERVAL: Duration = Duration::from_secs(5);
const QUIESCENT_STABILITY_WINDOW: Duration = Duration::from_secs(1);
const INITIAL_QUIESCENT_STABILITY_WINDOW: Duration = Duration::from_secs(3);
const CHECK_TIMEOUT: Duration = Duration::from_secs(300);
const CHECK_START_TIMEOUT: Duration = Duration::from_secs(30);
const CHECK_SETTLE_TIMEOUT: Duration = Duration::from_millis(500);
const FLYCHECK_PROGRESS_TOKEN_PREFIX: &str = "rust-analyzer/flycheck/";

/// Sends MCP progress while a tool waits for rust-analyzer's workspace index.
pub struct McpProgressReporter {
    peer: Peer<RoleServer>,
    token: Option<ProgressToken>,
    next_progress: AtomicU64,
}

impl McpProgressReporter {
    pub fn from_context(context: &RequestContext<RoleServer>) -> Self {
        Self {
            peer: context.peer.clone(),
            token: context.meta.get_progress_token(),
            next_progress: AtomicU64::new(1),
        }
    }

    pub async fn report(&self, message: impl Into<String>) {
        let Some(token) = self.token.clone() else {
            return;
        };
        let progress = self.next_progress.fetch_add(1, Ordering::Relaxed) as f64;
        let notification = ProgressNotificationParam::new(token, progress).with_message(message);
        if let Err(error) = self.peer.notify_progress(notification).await {
            tracing::debug!(error = %error, "could not report rust-analyzer indexing progress to MCP client");
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RustAnalyzerStatus {
    health: String,
    quiescent: bool,
    message: Option<String>,
}

#[derive(Debug)]
struct LspServerError {
    method: String,
    details: String,
}

impl std::fmt::Display for LspServerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "rust-analyzer request {} failed: {}",
            self.method, self.details
        )
    }
}

impl std::error::Error for LspServerError {}

#[derive(Clone)]
pub struct RustAnalyzerBridge {
    project_root: Arc<std::sync::RwLock<PathBuf>>,
    config: aimer_hot_reload::config::RustAnalyzerConfig,
    session: Arc<Mutex<Option<LspSession>>>,
    actions: Arc<Mutex<HashMap<String, Value>>>,
    previews: Arc<Mutex<HashMap<String, EditPreview>>>,
}

impl RustAnalyzerBridge {
    pub fn new(
        project_root: PathBuf,
        config: aimer_hot_reload::config::RustAnalyzerConfig,
    ) -> Self {
        Self {
            project_root: Arc::new(std::sync::RwLock::new(project_root)),
            config,
            session: Arc::new(Mutex::new(None)),
            actions: Arc::new(Mutex::new(HashMap::new())),
            previews: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Return the currently selected Rust workspace root.
    pub fn project_root(&self) -> PathBuf {
        self.project_root
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Whether Cargo subprocesses may use the network for this MCP session.
    pub fn allow_network(&self) -> bool {
        self.config.allow_network
    }

    /// Start or reuse rust-analyzer and wait until workspace indexing is ready.
    pub async fn start(
        &self,
        requested_project_path: Option<&Path>,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<Value> {
        let mut session = self.session.lock().await;
        let current_root = self.project_root();
        let requested_root = match requested_project_path {
            Some(path) if path.is_absolute() => path.to_path_buf(),
            Some(path) => current_root.join(path),
            None => current_root.clone(),
        };
        let requested_root = requested_root.canonicalize().with_context(|| {
            format!("resolving Rust project path '{}'", requested_root.display())
        })?;
        if !requested_root.is_dir() {
            bail!(
                "Rust project path '{}' is not a directory",
                requested_root.display()
            );
        }

        let root_changed = requested_root != current_root;
        if root_changed {
            // Dropping the prior session terminates its owned rust-analyzer process.
            *session = None;
            *self
                .project_root
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = requested_root.clone();
            self.actions.lock().await.clear();
            self.previews.lock().await.clear();
        }
        let mut started = session.is_none();
        let analyzer_exited = session
            .as_mut()
            .map(|analyzer| {
                analyzer
                    .child
                    .try_wait()
                    .context("checking existing rust-analyzer process")
                    .map(|status| status.is_some())
            })
            .transpose()?
            .unwrap_or(false);
        if analyzer_exited {
            *session = None;
            started = true;
        }
        self.ensure_session_started(&mut session, progress).await?;
        self.wait_session_until_ready(&mut session, progress)
            .await?;

        let child_status = session
            .as_mut()
            .expect("session initialized")
            .child
            .try_wait()
            .context("checking rust-analyzer process after startup")?;
        if let Some(status) = child_status {
            *session = None;
            bail!("rust-analyzer exited after workspace indexing: {status}");
        }

        let analyzer = session.as_ref().expect("session initialized");
        let health = analyzer
            .server_status
            .as_ref()
            .map_or("unknown", |status| status.health.as_str());
        let message = analyzer
            .server_status
            .as_ref()
            .and_then(|status| status.message.clone());
        Ok(json!({
            "status": "ready",
            "started": started,
            "workspace_ready": analyzer.workspace_ready,
            "health": health,
            "message": message,
            "process_id": analyzer.child.id(),
            "project_root": requested_root.display().to_string()
        }))
    }

    pub async fn request(
        &self,
        method: &str,
        params: Value,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<Value> {
        let mut session = self.session.lock().await;
        self.ensure_session_started(&mut session, progress).await?;
        self.wait_session_until_ready(&mut session, progress)
            .await?;
        let analyzer = session.as_mut().expect("session initialized");
        let result = analyzer.request(method, params, Some(progress)).await;
        if result
            .as_ref()
            .is_err_and(|error| error.downcast_ref::<LspServerError>().is_none())
        {
            *session = None;
        }
        result
    }

    pub async fn sync_document(
        &self,
        file: &Path,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<(String, String)> {
        let absolute = self.resolve_project_file(file, false)?;
        let contents = tokio::fs::read_to_string(&absolute)
            .await
            .with_context(|| format!("reading Rust source '{}' ", absolute.display()))?;
        let uri = path_to_uri(&absolute)?;
        let mut session = self.session.lock().await;
        self.ensure_session_started(&mut session, progress).await?;
        let analyzer = session.as_mut().expect("session initialized");
        let result = analyzer.sync_document(&uri, &contents).await;
        if result.is_err() {
            *session = None;
        }
        result?;
        self.wait_session_until_ready(&mut session, progress)
            .await?;
        Ok((uri, contents))
    }

    pub async fn diagnostics(
        &self,
        file: Option<&Path>,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<Value> {
        if let Some(file) = file {
            self.sync_document(file, progress).await?;
        }
        let mut session = self.session.lock().await;
        self.ensure_session_started(&mut session, progress).await?;
        self.wait_session_until_ready(&mut session, progress)
            .await?;
        let session = session.as_mut().expect("session initialized");
        let uri_filter = file
            .map(|file| self.resolve_project_file(file, false))
            .transpose()?
            .map(|path| path_to_uri(&path))
            .transpose()?;
        let project_root = self
            .project_root()
            .canonicalize()
            .context("resolving selected Rust project root")?;
        Ok(session.diagnostics_json(&project_root, uri_filter.as_deref()))
    }

    pub async fn run_check(
        &self,
        file: Option<&Path>,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<Value> {
        let document_uri = if let Some(file) = file {
            Some(self.sync_document(file, progress).await?.0)
        } else {
            None
        };
        let mut session = self.session.lock().await;
        self.ensure_session_started(&mut session, progress).await?;
        self.wait_session_until_ready(&mut session, progress)
            .await?;
        let result = session
            .as_mut()
            .expect("session initialized")
            .run_flycheck(document_uri.as_deref())
            .await;
        if let Err(error) = result {
            let analyzer_exited = session
                .as_mut()
                .and_then(|analyzer| analyzer.child.try_wait().ok().flatten())
                .is_some();
            if analyzer_exited {
                *session = None;
            }
            return Err(error).context("running rust-analyzer Clippy flycheck");
        }
        let session = session.as_mut().expect("session initialized");
        let project_root = self.project_root();
        Ok(flycheck_result_json(
            session.diagnostics_json(&project_root, None),
        ))
    }

    pub async fn get_code_actions(
        &self,
        file: &Path,
        range: Value,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<Value> {
        let (uri, _) = self.sync_document(file, progress).await?;
        let mut session_guard = self.session.lock().await;
        self.ensure_session_started(&mut session_guard, progress)
            .await?;
        self.wait_session_until_ready(&mut session_guard, progress)
            .await?;
        let result = {
            let session = session_guard.as_mut().expect("session initialized");
            let diagnostics = session.diagnostics.get(&uri).cloned().unwrap_or_default();
            session
                .request(
                    "textDocument/codeAction",
                    json!({
                        "textDocument": { "uri": uri },
                        "range": range,
                        "context": { "diagnostics": diagnostics }
                    }),
                    Some(progress),
                )
                .await
        };
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                if error.downcast_ref::<LspServerError>().is_none() {
                    *session_guard = None;
                }
                return Err(error);
            }
        };
        drop(session_guard);
        let actions = result.as_array().cloned().unwrap_or_default();
        let mut action_store = self.actions.lock().await;
        action_store.clear();
        let results = actions
            .into_iter()
            .take(MAX_RESULTS)
            .filter_map(|action| {
                let title = action.get("title")?.as_str()?.to_owned();
                let id = Uuid::new_v4().to_string();
                action_store.insert(id.clone(), action.clone());
                Some(json!({
                    "action_id": id,
                    "title": title,
                    "kind": action.get("kind").cloned().unwrap_or(Value::Null)
                }))
            })
            .collect::<Vec<_>>();
        Ok(json!({ "results": results, "truncated": actions_len_exceeds(&result, MAX_RESULTS) }))
    }

    pub async fn prepare_rename(
        &self,
        file: &Path,
        line: u32,
        column: u32,
        new_name: &str,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<Value> {
        let (uri, contents) = self.sync_document(file, progress).await?;
        let position = position_from_source(&contents, line, column)?;
        let old_name = identifier_at_lsp_position(&contents, &position).ok();
        let edit = self
            .request(
                "textDocument/rename",
                json!({
                    "textDocument": { "uri": uri },
                    "position": position,
                    "newName": new_name
                }),
                progress,
            )
            .await?;
        self.create_rename_preview(edit, old_name.as_deref()).await
    }

    pub async fn prepare_code_action(
        &self,
        action_id: &str,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<Value> {
        let action = self
            .actions
            .lock()
            .await
            .get(action_id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown or expired code action ID"))?;
        let action = if action.get("edit").is_none() {
            self.request("codeAction/resolve", action, progress).await?
        } else {
            action
        };
        let edit = action.get("edit").cloned().ok_or_else(|| {
            anyhow!("this code action does not provide a previewable workspace edit")
        })?;
        self.create_preview(edit).await
    }

    pub async fn prepare_format(
        &self,
        file: &Path,
        range: Option<Value>,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<Value> {
        let (uri, _) = self.sync_document(file, progress).await?;
        let (method, params) = match range {
            Some(range) => (
                "textDocument/rangeFormatting",
                json!({
                    "textDocument": { "uri": uri },
                    "range": range,
                    "options": { "tabSize": 4, "insertSpaces": true }
                }),
            ),
            None => (
                "textDocument/formatting",
                json!({
                    "textDocument": { "uri": uri },
                    "options": { "tabSize": 4, "insertSpaces": true }
                }),
            ),
        };
        let edits = self.request(method, params, progress).await?;
        let edit = json!({ "changes": { uri: edits } });
        self.create_preview(edit).await
    }

    pub async fn apply_preview(&self, preview_id: &str) -> anyhow::Result<Value> {
        let preview = {
            let mut previews = self.previews.lock().await;
            let existing = previews
                .get(preview_id)
                .ok_or_else(|| anyhow!("unknown, expired, or already applied edit preview"))?;
            if !existing.applicable {
                bail!("edit preview cannot be applied because manifest validation failed");
            }
            previews
                .remove(preview_id)
                .expect("preview was just checked")
        };
        if preview.created_at.elapsed() > Duration::from_secs(300) {
            bail!("edit preview expired; prepare a new preview before applying");
        }
        for file in &preview.files {
            let current = tokio::fs::read(&file.path)
                .await
                .with_context(|| format!("reading '{}' before applying edit", file.relative))?;
            if current != file.before {
                bail!("edit preview is stale because '{}' changed", file.relative);
            }
        }

        let mut applied: Vec<&PreviewFile> = Vec::new();
        for file in &preview.files {
            let result = replace_file_atomically(&file.path, &file.after).await;
            if let Err(error) = result {
                let mut rollback_errors = Vec::new();
                for previous in applied.iter().rev() {
                    if let Err(rollback_error) =
                        replace_file_atomically(&previous.path, &previous.before).await
                    {
                        rollback_errors.push(format!("{}: {rollback_error}", previous.relative));
                    }
                }
                if !rollback_errors.is_empty() {
                    return Err(error).with_context(|| {
                        format!(
                            "workspace edit failed and rollback also failed for {}",
                            rollback_errors.join(", ")
                        )
                    });
                }
                return Err(error)
                    .with_context(|| format!("applying workspace edit at '{}'", file.relative));
            }
            applied.push(file);
        }

        let contains_manifest = preview.files.iter().any(|file| {
            file.path
                .file_name()
                .is_some_and(|name| name == "Cargo.toml")
        });
        let mut workspace_reload = None;
        if let Some(session) = self.session.lock().await.as_mut() {
            for file in &preview.files {
                if file
                    .path
                    .extension()
                    .is_none_or(|extension| extension != "rs")
                {
                    continue;
                }
                if let Ok(uri) = path_to_uri(&file.path) {
                    let _ = session
                        .sync_document(&uri, std::str::from_utf8(&file.after).unwrap_or_default())
                        .await;
                }
            }
            if contains_manifest {
                workspace_reload = Some(
                    session
                        .request("rust-analyzer/reloadWorkspace", Value::Null, None)
                        .await
                        .map(|_| "requested".to_owned())
                        .unwrap_or_else(|error| format!("failed: {error:#}")),
                );
            }
        }
        let mut result = json!({
            "applied": true,
            "files": preview.files.iter().map(|file| file.relative.clone()).collect::<Vec<_>>()
        });
        if let Some(validation) = &preview.manifest_validation {
            result["manifest_validation"] = validation.clone();
        }
        if let Some(reload) = workspace_reload {
            result["workspace_reload"] = json!(reload);
        }
        if let Some(report) = &preview.macro_reference_candidates {
            result["macro_reference_candidates"] = macro_reference_report_json(report);
        }
        Ok(result)
    }

    async fn ensure_session_started(
        &self,
        session: &mut Option<LspSession>,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<()> {
        if session.is_none() {
            progress
                .report(
                    "Starting rust-analyzer; this tool will wait for workspace indexing to finish.",
                )
                .await;
            *session = Some(
                LspSession::start(&self.project_root(), &self.config, progress)
                    .await
                    .context("starting rust-analyzer")?,
            );
        }
        Ok(())
    }

    async fn wait_session_until_ready(
        &self,
        session: &mut Option<LspSession>,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<()> {
        let result = session
            .as_mut()
            .expect("session initialized")
            .wait_until_ready(progress)
            .await;
        if result.is_err()
            && session
                .as_mut()
                .and_then(|analyzer| analyzer.child.try_wait().ok().flatten())
                .is_some()
        {
            *session = None;
        }
        result
    }

    async fn create_preview(&self, workspace_edit: Value) -> anyhow::Result<Value> {
        let project_root = self.project_root();
        let files = workspace_edit_files(&project_root, &workspace_edit).await?;
        self.finish_preview(files, None).await
    }

    /// Create a stale-safe preview for a manifest candidate not produced by rust-analyzer.
    pub async fn prepare_manifest_content_preview(
        &self,
        manifest: &Path,
        before: &[u8],
        after: &[u8],
        validation: Value,
        applicable: bool,
    ) -> anyhow::Result<Value> {
        let project_root = self
            .project_root()
            .canonicalize()
            .context("resolving selected Rust project root")?;
        let path = self.resolve_project_file(manifest, false)?;
        anyhow::ensure!(
            path.file_name().is_some_and(|name| name == "Cargo.toml"),
            "manifest edit preview must target Cargo.toml"
        );
        let current = tokio::fs::read(&path)
            .await
            .with_context(|| format!("reading manifest '{}' for preview", path.display()))?;
        anyhow::ensure!(
            current == before,
            "manifest changed while Cargo validation was running; prepare a fresh preview"
        );
        let relative = path
            .strip_prefix(&project_root)
            .context("manifest is outside the selected Rust project")?
            .to_string_lossy()
            .replace('\\', "/");
        let (diff, diff_truncated) = unified_diff_with_flag(before, after);
        let file = PreviewFile {
            path,
            relative,
            before: before.to_vec(),
            after: after.to_vec(),
            diff_truncated,
        };
        self.finish_preview_with_validation(vec![file], validation, applicable)
            .await
            .map(|mut result| {
                result["files"][0]["diff"] = json!(diff);
                result
            })
    }

    async fn create_rename_preview(
        &self,
        workspace_edit: Value,
        old_name: Option<&str>,
    ) -> anyhow::Result<Value> {
        let project_root = self.project_root();
        let files = workspace_edit_files(&project_root, &workspace_edit).await?;
        let report = find_unedited_macro_references(&project_root, &files, old_name).await;
        self.finish_preview(files, Some(report)).await
    }

    async fn finish_preview(
        &self,
        files: Vec<PreviewFile>,
        macro_reference_candidates: Option<MacroReferenceReport>,
    ) -> anyhow::Result<Value> {
        self.finish_preview_inner(files, macro_reference_candidates, None, true)
            .await
    }

    async fn finish_preview_with_validation(
        &self,
        files: Vec<PreviewFile>,
        validation: Value,
        applicable: bool,
    ) -> anyhow::Result<Value> {
        self.finish_preview_inner(files, None, Some(validation), applicable)
            .await
    }

    async fn finish_preview_inner(
        &self,
        files: Vec<PreviewFile>,
        macro_reference_candidates: Option<MacroReferenceReport>,
        manifest_validation: Option<Value>,
        applicable: bool,
    ) -> anyhow::Result<Value> {
        if files.is_empty() {
            let mut result = json!({ "changed": false, "files": [] });
            if let Some(report) = &macro_reference_candidates {
                result["macro_reference_candidates"] = macro_reference_report_json(report);
            }
            if let Some(validation) = &manifest_validation {
                result["manifest_validation"] = validation.clone();
                result["applicable"] = json!(applicable);
            }
            return Ok(result);
        }
        let preview_id = Uuid::new_v4().to_string();
        let file_views = files
            .iter()
            .map(|file| {
                json!({
                    "file": file.relative,
                    "diff": unified_diff(&file.before, &file.after),
                    "truncated": file.diff_truncated
                })
            })
            .collect::<Vec<_>>();
        let mut previews = self.previews.lock().await;
        previews.retain(|_, preview| preview.created_at.elapsed() < Duration::from_secs(300));
        if previews.len() >= 32 {
            if let Some(oldest) = previews.keys().next().cloned() {
                previews.remove(&oldest);
            }
        }
        previews.insert(
            preview_id.clone(),
            EditPreview {
                files,
                macro_reference_candidates: macro_reference_candidates.clone(),
                manifest_validation: manifest_validation.clone(),
                applicable,
                created_at: Instant::now(),
            },
        );
        let mut result = json!({ "changed": true, "preview_id": preview_id, "files": file_views });
        if let Some(report) = &macro_reference_candidates {
            result["macro_reference_candidates"] = macro_reference_report_json(report);
        }
        if let Some(validation) = &manifest_validation {
            result["manifest_validation"] = validation.clone();
            result["applicable"] = json!(applicable);
        }
        Ok(result)
    }

    fn resolve_project_file(&self, file: &Path, allow_missing: bool) -> anyhow::Result<PathBuf> {
        let project_root = self
            .project_root()
            .canonicalize()
            .context("resolving selected Rust project root")?;
        let candidate = if file.is_absolute() {
            file.to_path_buf()
        } else {
            project_root.join(file)
        };
        let resolved = if allow_missing && !candidate.exists() {
            let parent = candidate
                .parent()
                .ok_or_else(|| anyhow!("file path has no parent"))?
                .canonicalize()
                .context("resolving parent directory")?;
            parent.join(
                candidate
                    .file_name()
                    .ok_or_else(|| anyhow!("file path has no name"))?,
            )
        } else {
            candidate
                .canonicalize()
                .with_context(|| format!("resolving project file '{}' ", candidate.display()))?
        };
        if !resolved.starts_with(&project_root) {
            bail!("file path must stay inside the selected project");
        }
        Ok(resolved)
    }
}

pub fn normalize_result(root: &Path, value: &Value) -> Value {
    normalize_lsp_value(root, value)
}

pub fn normalize_bounded(root: &Path, value: &Value) -> Value {
    if let Some(items) = value.as_array() {
        json!({
            "results": items.iter().take(MAX_RESULTS).map(|item| normalize_lsp_value(root, item)).collect::<Vec<_>>(),
            "truncated": items.len() > MAX_RESULTS
        })
    } else {
        normalize_lsp_value(root, value)
    }
}

pub async fn search_text(
    root: &Path,
    query: &str,
    file_glob: Option<&str>,
    case_sensitive: bool,
    limit: usize,
) -> anyhow::Result<Value> {
    let query = query.trim();
    if query.is_empty() {
        bail!("query must not be empty");
    }
    if query.len() > 512 {
        bail!("query must be at most 512 bytes");
    }
    if !(1..=MAX_RESULTS).contains(&limit) {
        bail!("limit must be between 1 and {MAX_RESULTS}");
    }
    let candidates = match Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()
        .await
    {
        Ok(output) if output.status.success() => output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| root.join(String::from_utf8_lossy(path).as_ref()))
            .collect::<Vec<_>>(),
        _ => {
            let mut files = Vec::new();
            collect_search_files(root, root, &mut files)?;
            files
        }
    };

    let needle = if case_sensitive {
        query.to_owned()
    } else {
        query.to_ascii_lowercase()
    };
    let mut results = Vec::new();
    let mut truncated = false;
    for path in candidates {
        if path_glob_matches(file_glob, root, &path) == Some(false) {
            continue;
        }
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if !canonical.starts_with(root) {
            continue;
        }
        let Ok(metadata) = tokio::fs::metadata(&canonical).await else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > 1024 * 1024 {
            continue;
        }
        let Ok(contents) = tokio::fs::read_to_string(&canonical).await else {
            continue;
        };
        for (line_index, line) in contents.lines().enumerate() {
            let haystack = if case_sensitive {
                line.to_owned()
            } else {
                line.to_ascii_lowercase()
            };
            if let Some(byte_column) = haystack.find(&needle) {
                if results.len() == limit {
                    truncated = true;
                    break;
                }
                let column = line[..byte_column.min(line.len())].chars().count() + 1;
                results.push(json!({
                    "file": canonical.strip_prefix(root)?.to_string_lossy().replace('\\', "/"),
                    "line": line_index + 1,
                    "column": column,
                    "text": line.trim().chars().take(500).collect::<String>()
                }));
            }
        }
        if truncated {
            break;
        }
    }
    Ok(json!({ "query": query, "results": results, "truncated": truncated }))
}

fn path_glob_matches(pattern: Option<&str>, root: &Path, path: &Path) -> Option<bool> {
    let pattern = pattern?;
    let relative = path
        .strip_prefix(root)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    Some(simple_glob(pattern, &relative))
}

fn simple_glob(pattern: &str, value: &str) -> bool {
    fn matches(
        pattern: &[char],
        value: &[char],
        pi: usize,
        vi: usize,
        memo: &mut HashMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(result) = memo.get(&(pi, vi)) {
            return *result;
        }
        let result = if pi == pattern.len() {
            vi == value.len()
        } else if pattern[pi] == '*' {
            let double = pattern.get(pi + 1) == Some(&'*');
            let next = pi + if double { 2 } else { 1 };
            if double
                && pattern.get(next) == Some(&'/')
                && matches(pattern, value, next + 1, vi, memo)
            {
                true
            } else {
                let mut result = false;
                let mut end = vi;
                while end <= value.len() {
                    if matches(pattern, value, next, end, memo) {
                        result = true;
                        break;
                    }
                    if end == value.len() || (!double && value[end] == '/') {
                        break;
                    }
                    end += 1;
                }
                result
            }
        } else if pattern[pi] == '?' {
            vi < value.len() && value[vi] != '/' && matches(pattern, value, pi + 1, vi + 1, memo)
        } else {
            vi < value.len()
                && pattern[pi] == value[vi]
                && matches(pattern, value, pi + 1, vi + 1, memo)
        };
        memo.insert((pi, vi), result);
        result
    }

    let pattern = pattern.replace('\\', "/").chars().collect::<Vec<_>>();
    let value = value.chars().collect::<Vec<_>>();
    matches(&pattern, &value, 0, 0, &mut HashMap::new())
}

fn collect_search_files(
    root: &Path,
    current: &Path,
    output: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(current)
        .with_context(|| format!("reading directory '{}' ", current.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry.file_type()?;
        if metadata.is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            if name == ".git" || name == "target" || name == ".aimer" || name.starts_with('.') {
                continue;
            }
            collect_search_files(root, &path, output)?;
        } else if metadata.is_file() && path.starts_with(root) {
            output.push(path);
        }
    }
    Ok(())
}

#[derive(Debug)]
struct PreviewFile {
    path: PathBuf,
    relative: String,
    before: Vec<u8>,
    after: Vec<u8>,
    diff_truncated: bool,
}

#[derive(Debug)]
struct EditPreview {
    files: Vec<PreviewFile>,
    macro_reference_candidates: Option<MacroReferenceReport>,
    manifest_validation: Option<Value>,
    applicable: bool,
    created_at: Instant,
}

#[derive(Clone, Debug)]
struct MacroReferenceReport {
    symbol: Option<String>,
    candidates: Vec<MacroReferenceCandidate>,
    scanned_files: usize,
    scan_complete: bool,
    truncated: bool,
    issue_count: usize,
}

#[derive(Clone, Debug)]
struct MacroReferenceCandidate {
    file: String,
    line: usize,
    column: usize,
    macro_name: String,
    text: String,
}

struct LspSession {
    child: Child,
    stdin: ChildStdin,
    incoming: mpsc::Receiver<anyhow::Result<Value>>,
    next_id: AtomicU64,
    diagnostics: HashMap<String, Vec<Value>>,
    open_documents: HashMap<String, (String, i64)>,
    server_status: Option<RustAnalyzerStatus>,
    workspace_ready: bool,
}

fn rust_analyzer_configuration() -> Value {
    json!({
        "checkOnSave": false,
        "check": {
            "command": "clippy",
            "extraArgs": ["--", "-D", "warnings"]
        },
        "cargo": { "buildScripts": { "enable": true } },
        "procMacro": { "enable": true }
    })
}

impl LspSession {
    async fn start(
        project_root: &Path,
        config: &aimer_hot_reload::config::RustAnalyzerConfig,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<Self> {
        let executable = config
            .binary_path
            .clone()
            .unwrap_or_else(|| PathBuf::from("rust-analyzer"));
        let mut command = Command::new(&executable);
        command
            .current_dir(project_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if config.allow_network {
            command.env("CARGO_NET_OFFLINE", "false");
        } else {
            command.env("CARGO_NET_OFFLINE", "true");
        }
        let mut child = command.spawn().with_context(|| {
            format!(
                "could not start '{}'; install rust-analyzer or set mcp.rust_analyzer.binary_path",
                executable.display()
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("rust-analyzer stdin was not available"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("rust-analyzer stdout was not available"))?;
        let (message_sender, incoming) = mpsc::channel(MAX_PENDING_LSP_MESSAGES);
        tokio::spawn(async move {
            let mut stdout = BufReader::new(stdout);
            loop {
                match read_lsp_message(&mut stdout).await {
                    Ok(message) => {
                        if message_sender.send(Ok(message)).await.is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = message_sender.send(Err(error)).await;
                        break;
                    }
                }
            }
        });
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(target: "aimer::rust_analyzer", "{line}");
                }
            });
        }
        let mut session = Self {
            child,
            stdin,
            incoming,
            next_id: AtomicU64::new(1),
            diagnostics: HashMap::new(),
            open_documents: HashMap::new(),
            server_status: None,
            workspace_ready: false,
        };
        let root_uri = path_to_uri(project_root)?;
        let initialize = session
            .request(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "rootUri": root_uri,
                    "rootPath": project_root.to_string_lossy(),
                    "workspaceFolders": [{ "uri": root_uri, "name": "aimer-project" }],
                    "capabilities": {
                        "workspace": {
                            "applyEdit": false,
                            "configuration": true,
                            "workspaceFolders": true,
                            "workspaceEdit": {
                                "documentChanges": true,
                                "resourceOperations": ["create", "rename", "delete"]
                            }
                        },
                        "textDocument": {
                            "publishDiagnostics": { "relatedInformation": true },
                            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                            "definition": { "linkSupport": true },
                            "codeAction": { "codeActionLiteralSupport": { "codeActionKind": { "valueSet": [""] } } },
                            "rename": { "prepareSupport": true },
                            "callHierarchy": { "dynamicRegistration": false }
                        },
                        "window": { "workDoneProgress": true },
                        "experimental": { "serverStatusNotification": true }
                    },
                    "initializationOptions": rust_analyzer_configuration()
                }),
                Some(progress),
            )
            .await?;
        if initialize.get("capabilities").is_none() {
            bail!("rust-analyzer initialize response omitted capabilities");
        }
        session.notify("initialized", json!({})).await?;
        Ok(session)
    }

    async fn request(
        &mut self,
        method: &str,
        params: Value,
        progress: Option<&McpProgressReporter>,
    ) -> anyhow::Result<Value> {
        // Keep our request IDs distinct from rust-analyzer's server-initiated
        // requests. Both peers use the same JSON-RPC connection, and rust-analyzer
        // uses numeric IDs for requests such as window/workDoneProgress/create.
        let id = format!("aimer-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        self.write_message(
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .await?;
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let mut next_progress_at = Instant::now() + INDEXING_PROGRESS_INTERVAL;
        loop {
            let now = Instant::now();
            if now >= deadline {
                bail!("timed out waiting for rust-analyzer request {method}");
            }
            let remaining = deadline.duration_since(now);
            let read_timeout = if progress.is_some() {
                remaining.min(next_progress_at.saturating_duration_since(now))
            } else {
                remaining
            };
            match timeout(read_timeout, self.read_message()).await {
                Ok(message) => {
                    let message = message?;
                    if message.get("method").is_none() && message.get("id") == Some(&json!(id)) {
                        if let Some(error) = message.get("error") {
                            return Err(LspServerError {
                                method: method.to_owned(),
                                details: error.to_string(),
                            }
                            .into());
                        }
                        return Ok(message.get("result").cloned().unwrap_or(Value::Null));
                    }
                    self.handle_message(message).await?;
                }
                Err(_) if Instant::now() >= deadline => {
                    bail!("timed out waiting for rust-analyzer request {method}");
                }
                Err(_) => {
                    if let Some(progress) = progress {
                        progress
                            .report(format!(
                                "rust-analyzer is processing LSP request {method}; wait for the tool result."
                            ))
                            .await;
                    }
                    next_progress_at = Instant::now() + INDEXING_PROGRESS_INTERVAL;
                }
            }
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> anyhow::Result<()> {
        self.write_message(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    async fn sync_document(&mut self, uri: &str, text: &str) -> anyhow::Result<()> {
        let change_version = match self.open_documents.get_mut(uri) {
            Some((previous, version)) if previous != text => {
                *version += 1;
                *previous = text.to_owned();
                self.workspace_ready = false;
                Some(*version)
            }
            Some(_) => return Ok(()),
            None => {
                self.open_documents
                    .insert(uri.to_owned(), (text.to_owned(), 1));
                self.workspace_ready = false;
                self.notify(
                    "textDocument/didOpen",
                    json!({ "textDocument": { "uri": uri, "languageId": "rust", "version": 1, "text": text } }),
                ).await?;
                return Ok(());
            }
        };
        if let Some(version) = change_version {
            self.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": uri, "version": version },
                    "contentChanges": [{ "text": text }]
                }),
            )
            .await?;
        }
        Ok(())
    }

    async fn wait_until_ready(&mut self, progress: &McpProgressReporter) -> anyhow::Result<()> {
        if self.workspace_ready {
            loop {
                match timeout(INDEXING_POLL_INTERVAL, self.read_message()).await {
                    Ok(Ok(message)) => {
                        self.handle_message(message).await?;
                        if !self.workspace_ready {
                            break;
                        }
                    }
                    Ok(Err(error)) => return Err(error),
                    Err(_) => return Ok(()),
                }
            }
        }

        let started_at = Instant::now();
        let mut quiescent_since = None;
        let mut saw_indexing = false;
        let mut last_reported_status = None;
        let mut last_progress_at = Instant::now();
        progress
            .report("rust-analyzer is loading and indexing the workspace; wait for this tool to finish.")
            .await;

        loop {
            let now = Instant::now();
            if now.duration_since(started_at) >= INDEXING_TIMEOUT {
                let status = self
                    .server_status
                    .as_ref()
                    .map_or_else(|| "no server status received".to_owned(), format_status);
                bail!(
                    "rust-analyzer did not report a ready workspace within {} seconds ({status}); retry after indexing completes",
                    INDEXING_TIMEOUT.as_secs()
                );
            }

            if let Some(status) = self.server_status.as_ref() {
                if status.quiescent && status.health == "error" {
                    bail!(
                        "rust-analyzer finished loading with health=error: {}; semantic results may be incomplete",
                        status
                            .message
                            .as_deref()
                            .unwrap_or("no status message provided")
                    );
                }

                if status.quiescent {
                    let quiescent_since = quiescent_since.get_or_insert(now);
                    let stability_window = if saw_indexing {
                        QUIESCENT_STABILITY_WINDOW
                    } else {
                        INITIAL_QUIESCENT_STABILITY_WINDOW
                    };
                    if now.duration_since(*quiescent_since) >= stability_window {
                        self.workspace_ready = true;
                        let ready_message = if status.health == "warning" {
                            format!(
                                "rust-analyzer workspace is ready with a warning: {}",
                                status
                                    .message
                                    .as_deref()
                                    .unwrap_or("some project information is unavailable")
                            )
                        } else {
                            "rust-analyzer workspace indexing is complete; continuing the tool request.".to_owned()
                        };
                        progress.report(ready_message).await;
                        return Ok(());
                    }
                } else {
                    saw_indexing = true;
                    quiescent_since = None;
                }

                if last_reported_status.as_ref() != Some(status) {
                    progress.report(status_progress_message(status)).await;
                    last_reported_status = Some(status.clone());
                    last_progress_at = now;
                }
            }

            if now.duration_since(last_progress_at) >= INDEXING_PROGRESS_INTERVAL {
                let message = self.server_status.as_ref().map_or_else(
                    || {
                        "Still waiting for rust-analyzer to report workspace indexing status."
                            .to_owned()
                    },
                    |status| {
                        format!(
                            "Still waiting for rust-analyzer: {}",
                            status_progress_message(status)
                        )
                    },
                );
                progress.report(message).await;
                last_progress_at = now;
            }

            match timeout(INDEXING_POLL_INTERVAL, self.read_message()).await {
                Ok(message) => self.handle_message(message?).await?,
                Err(_) => {}
            }
        }
    }

    async fn run_flycheck(&mut self, uri: Option<&str>) -> anyhow::Result<()> {
        self.notify(
            "rust-analyzer/runFlycheck",
            json!({ "textDocument": uri.map(|uri| json!({ "uri": uri })).unwrap_or(Value::Null) }),
        )
        .await
        .context("check_runner_unavailable: could not start rust-analyzer Clippy flycheck")?;
        timeout(CHECK_TIMEOUT, async {
            let started_at = Instant::now();
            let mut saw_flycheck = false;
            let mut active_flychecks = HashSet::new();
            let mut last_flycheck_end = None;
            loop {
                let message = if saw_flycheck && active_flychecks.is_empty() {
                    let elapsed = last_flycheck_end
                        .map(|ended_at: Instant| ended_at.elapsed())
                        .unwrap_or_default();
                    let remaining = CHECK_SETTLE_TIMEOUT.saturating_sub(elapsed);
                    match timeout(remaining, self.read_message()).await {
                        Ok(message) => message?,
                        Err(_) => return Ok(()),
                    }
                } else if !saw_flycheck {
                    let remaining = CHECK_START_TIMEOUT.saturating_sub(started_at.elapsed());
                    match timeout(remaining, self.read_message()).await {
                        Ok(message) => message?,
                        Err(_) => bail!(
                            "check_start_timeout: rust-analyzer did not start flycheck within {} seconds; verify Cargo workspace discovery, target access, and that flycheck is enabled",
                            CHECK_START_TIMEOUT.as_secs()
                        ),
                    }
                } else {
                    self.read_message().await?
                };

                if let Some((token, kind)) = progress_event(&message) {
                    if token
                        .strip_prefix("string:")
                        .is_some_and(|token| token.starts_with(FLYCHECK_PROGRESS_TOKEN_PREFIX))
                    {
                        match kind.as_str() {
                            "begin" => {
                                saw_flycheck = true;
                                active_flychecks.insert(token);
                                last_flycheck_end = None;
                            }
                            "end" => {
                                saw_flycheck = true;
                                active_flychecks.remove(&token);
                                if active_flychecks.is_empty() {
                                    last_flycheck_end = Some(Instant::now());
                                }
                            }
                            _ => {}
                        }
                    }
                }
                self.handle_message(message).await?;
            }
        })
        .await
        .map_err(|_| {
            anyhow!(
                "check_timeout: rust-analyzer Clippy flycheck exceeded {} seconds while waiting for completion",
                CHECK_TIMEOUT.as_secs()
            )
        })?
    }

    fn diagnostics_json(&self, root: &Path, uri_filter: Option<&str>) -> Value {
        let mut all = Vec::new();
        for (uri, diagnostics) in &self.diagnostics {
            if uri_filter.is_some_and(|filter| filter != uri) {
                continue;
            }
            let file = uri_to_relative(root, uri).unwrap_or_else(|| uri.to_owned());
            for diagnostic in diagnostics {
                all.push(json!({
                    "file": file,
                    "range": normalize_range(diagnostic.get("range")),
                    "severity": diagnostic
                        .get("severity")
                        .and_then(Value::as_u64)
                        .map(diagnostic_severity_name)
                        .map(|value| Value::String(value.to_owned()))
                        .unwrap_or(Value::Null),
                    "code": diagnostic.get("code").cloned().unwrap_or(Value::Null),
                    "source": diagnostic.get("source").cloned().unwrap_or(Value::Null),
                    "message": diagnostic.get("message").cloned().unwrap_or(Value::Null),
                    "related_information": diagnostic.get("relatedInformation").cloned().unwrap_or(Value::Null)
                }));
            }
        }
        let truncated = all.len() > MAX_RESULTS;
        all.truncate(MAX_RESULTS);
        json!({ "diagnostics": all, "truncated": truncated })
    }

    async fn handle_message(&mut self, message: Value) -> anyhow::Result<()> {
        if let Some(method) = message.get("method").and_then(Value::as_str) {
            match method {
                "experimental/serverStatus" => {
                    if let Some(params) = message.get("params") {
                        if let (Some(health), Some(quiescent)) = (
                            params.get("health").and_then(Value::as_str),
                            params.get("quiescent").and_then(Value::as_bool),
                        ) {
                            let status = RustAnalyzerStatus {
                                health: health.to_owned(),
                                quiescent,
                                message: params
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned),
                            };
                            if !status.quiescent {
                                self.workspace_ready = false;
                            }
                            self.server_status = Some(status);
                        }
                    }
                }
                "textDocument/publishDiagnostics" => {
                    if let Some(params) = message.get("params") {
                        if let (Some(uri), Some(diagnostics)) = (
                            params.get("uri").and_then(Value::as_str),
                            params.get("diagnostics").and_then(Value::as_array),
                        ) {
                            self.diagnostics.insert(uri.to_owned(), diagnostics.clone());
                        }
                    }
                }
                "$/progress" => {
                    // Flycheck progress is tracked locally by `run_flycheck` so
                    // unrelated LSP work cannot hold a check open.
                }
                "workspace/diagnostic/refresh" => {
                    if let Some(id) = message.get("id") {
                        self.write_message(&json!({ "jsonrpc": "2.0", "id": id, "result": null }))
                            .await?;
                    }
                }
                _ => {
                    if let Some(id) = message.get("id") {
                        let result = match method {
                            "workspace/configuration" => {
                                let count = message
                                    .pointer("/params/items")
                                    .and_then(Value::as_array)
                                    .map_or(0, Vec::len);
                                Value::Array(
                                    (0..count).map(|_| rust_analyzer_configuration()).collect(),
                                )
                            }
                            "window/workDoneProgress/create"
                            | "client/registerCapability"
                            | "client/unregisterCapability" => Value::Null,
                            "workspace/applyEdit" => {
                                json!({ "applied": false, "failureReason": "Aimer previews workspace edits before applying them" })
                            }
                            "window/showMessageRequest" => Value::Null,
                            _ => {
                                self.write_message(&json!({
                                    "jsonrpc": "2.0", "id": id,
                                    "error": { "code": -32601, "message": format!("unsupported rust-analyzer client request {method}") }
                                })).await?;
                                return Ok(());
                            }
                        };
                        self.write_message(
                            &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                        )
                        .await?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn read_message(&mut self) -> anyhow::Result<Value> {
        match self.incoming.recv().await {
            Some(Ok(message)) => Ok(message),
            Some(Err(error)) => {
                let status = self.child.try_wait().ok().flatten();
                Err(error).with_context(|| {
                    format!(
                        "rust-analyzer output closed{}",
                        status
                            .map(|status| format!(" ({status})"))
                            .unwrap_or_default()
                    )
                })
            }
            None => bail!("rust-analyzer output reader stopped"),
        }
    }

    async fn write_message(&mut self, value: &Value) -> anyhow::Result<()> {
        let body = serde_json::to_vec(value)?;
        if body.len() > MAX_LSP_MESSAGE_BYTES {
            bail!("outgoing rust-analyzer LSP message exceeds the size limit");
        }
        self.stdin
            .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
            .await?;
        self.stdin.write_all(&body).await?;
        self.stdin.flush().await?;
        Ok(())
    }
}

async fn read_lsp_message(stdout: &mut BufReader<ChildStdout>) -> anyhow::Result<Value> {
    let mut content_length = None;
    loop {
        let mut line = Vec::new();
        let count = stdout.read_until(b'\n', &mut line).await?;
        if count == 0 {
            bail!("rust-analyzer closed its output");
        }
        if line == b"\r\n" || line == b"\n" {
            break;
        }
        if let Some((name, value)) = std::str::from_utf8(&line)
            .ok()
            .and_then(|line| line.trim().split_once(':'))
        {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .context("invalid LSP Content-Length")?,
                );
            }
        }
    }
    let length = content_length
        .ok_or_else(|| anyhow!("rust-analyzer LSP message omitted Content-Length"))?;
    if length > MAX_LSP_MESSAGE_BYTES {
        bail!("rust-analyzer LSP message exceeds the {MAX_LSP_MESSAGE_BYTES}-byte limit");
    }
    let mut body = vec![0; length];
    stdout.read_exact(&mut body).await?;
    serde_json::from_slice(&body).context("decoding rust-analyzer LSP message")
}

fn progress_event(message: &Value) -> Option<(String, String)> {
    if message.get("method")?.as_str()? != "$/progress" {
        return None;
    }
    let params = message.get("params")?;
    let token = match params.get("token")? {
        Value::String(token) => format!("string:{token}"),
        Value::Number(token) => format!("number:{token}"),
        _ => return None,
    };
    let kind = params.pointer("/value/kind")?.as_str()?.to_owned();
    Some((token, kind))
}

fn format_status(status: &RustAnalyzerStatus) -> String {
    format!(
        "health={}, quiescent={}, message={}",
        status.health,
        status.quiescent,
        status.message.as_deref().unwrap_or("none")
    )
}

fn status_progress_message(status: &RustAnalyzerStatus) -> String {
    let detail = status.message.as_deref().unwrap_or("no additional status");
    if status.quiescent {
        format!(
            "rust-analyzer reports no pending workspace work; confirming stable readiness (health={}): {detail}",
            status.health
        )
    } else {
        format!(
            "rust-analyzer is still indexing the workspace (health={}): {detail}",
            status.health
        )
    }
}

fn actions_len_exceeds(value: &Value, limit: usize) -> bool {
    value.as_array().is_some_and(|items| items.len() > limit)
}

fn path_to_uri(path: &Path) -> anyhow::Result<String> {
    reqwest::Url::from_file_path(path)
        .map(|url| url.to_string())
        .map_err(|_| anyhow!("cannot convert '{}' to a file URI", path.display()))
}

fn uri_to_relative(root: &Path, uri: &str) -> Option<String> {
    let path = reqwest::Url::parse(uri).ok()?.to_file_path().ok()?;
    let relative = path.strip_prefix(root).ok()?;
    Some(relative.to_string_lossy().replace('\\', "/"))
}

fn normalize_range(value: Option<&Value>) -> Value {
    let Some(value) = value else {
        return Value::Null;
    };
    let normalize_position = |position: Option<&Value>| -> Value {
        let Some(position) = position else {
            return Value::Null;
        };
        json!({
            "line": position.get("line").and_then(Value::as_u64).unwrap_or(0) + 1,
            "column": position.get("character").and_then(Value::as_u64).unwrap_or(0) + 1
        })
    };
    json!({ "start": normalize_position(value.get("start")), "end": normalize_position(value.get("end")) })
}

fn normalize_lsp_value(root: &Path, value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(MAX_RESULTS)
                .map(|item| normalize_lsp_value(root, item))
                .collect(),
        ),
        Value::Object(object) => {
            let mut normalized = Map::new();
            for (key, value) in object {
                if (key == "deprecated" && value == &Value::Bool(false))
                    || (key == "tags" && value.as_array().is_some_and(Vec::is_empty))
                {
                    continue;
                }
                let mapped_key = match key.as_str() {
                    "uri" => "file",
                    "targetUri" => "target_file",
                    "range" => "range",
                    "targetRange" => "target_range",
                    "targetSelectionRange" => "target_selection_range",
                    "selectionRange" => "selection_range",
                    other => other,
                };
                let mapped_value = match key.as_str() {
                    "uri" | "targetUri" => value
                        .as_str()
                        .and_then(|uri| uri_to_relative(root, uri))
                        .or_else(|| {
                            value.as_str().and_then(|uri| {
                                reqwest::Url::parse(uri)
                                    .ok()?
                                    .to_file_path()
                                    .ok()
                                    .map(|path| path.to_string_lossy().into_owned())
                            })
                        })
                        .map(|value| Value::String(value.to_owned()))
                        .unwrap_or_else(|| value.clone()),
                    "kind" if value.is_number() => value
                        .as_u64()
                        .map(symbol_kind_name)
                        .map(|value| Value::String(value.to_owned()))
                        .unwrap_or_else(|| value.clone()),
                    "severity" if value.is_number() => value
                        .as_u64()
                        .map(diagnostic_severity_name)
                        .map(|value| Value::String(value.to_owned()))
                        .unwrap_or_else(|| value.clone()),
                    "range" | "targetRange" | "targetSelectionRange" | "selectionRange" => {
                        normalize_range(Some(value))
                    }
                    "position" => {
                        let range = json!({ "start": value, "end": value });
                        normalize_range(Some(&range))
                            .get("start")
                            .cloned()
                            .unwrap_or(Value::Null)
                    }
                    _ => normalize_lsp_value(root, value),
                };
                normalized.insert(mapped_key.to_owned(), mapped_value);
                if value
                    .as_array()
                    .is_some_and(|items| items.len() > MAX_RESULTS)
                {
                    normalized.insert(format!("{mapped_key}_truncated"), Value::Bool(true));
                }
            }
            Value::Object(normalized)
        }
        _ => value.clone(),
    }
}

fn symbol_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "file",
        2 | 3 => "module",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        15 => "string",
        16 => "number",
        17 => "boolean",
        18 => "array",
        19 => "object",
        20 => "key",
        21 => "null",
        22 => "enum_member",
        23 => "struct",
        24 => "event",
        25 => "operator",
        26 => "type_parameter",
        _ => "other",
    }
}

fn flycheck_result_json(mut diagnostics: Value) -> Value {
    let items = diagnostics
        .get("diagnostics")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let error_count = items
        .iter()
        .filter(|item| item.get("severity").and_then(Value::as_str) == Some("error"))
        .count();
    let warning_count = items
        .iter()
        .filter(|item| item.get("severity").and_then(Value::as_str) == Some("warning"))
        .count();
    let Some(object) = diagnostics.as_object_mut() else {
        return json!({
            "operation": "cargo_clippy",
            "command": "clippy",
            "warnings_denied": true,
            "status": "completed",
            "cargo_exit_status": "not_reported_by_rust_analyzer",
            "diagnostic_summary": { "error_count": error_count, "warning_count": warning_count },
            "diagnostics": diagnostics,
            "truncated": false
        });
    };
    object.insert("operation".to_owned(), json!("cargo_clippy"));
    object.insert("command".to_owned(), json!("clippy"));
    object.insert("warnings_denied".to_owned(), json!(true));
    object.insert(
        "status".to_owned(),
        json!(if error_count > 0 || warning_count > 0 {
            "completed_with_errors"
        } else {
            "completed"
        }),
    );
    object.insert(
        "cargo_exit_status".to_owned(),
        json!("not_reported_by_rust_analyzer"),
    );
    object.insert(
        "diagnostic_summary".to_owned(),
        json!({ "error_count": error_count, "warning_count": warning_count }),
    );
    diagnostics
}

fn diagnostic_severity_name(severity: u64) -> &'static str {
    match severity {
        1 => "error",
        2 => "warning",
        3 => "information",
        4 => "hint",
        _ => "unknown",
    }
}

fn position_from_source(source: &str, line: u32, column: u32) -> anyhow::Result<Value> {
    if line == 0 || column == 0 {
        bail!("line and column are 1-based and must be greater than zero");
    }
    let line_text = source
        .lines()
        .nth((line - 1) as usize)
        .ok_or_else(|| anyhow!("line is outside the source file"))?;
    let requested_units = (column - 1) as usize;
    let mut units = 0;
    for ch in line_text.chars() {
        if units == requested_units {
            break;
        }
        units += ch.len_utf16();
        if units > requested_units {
            bail!("column splits a UTF-16 surrogate pair");
        }
    }
    if units != requested_units {
        bail!("column is outside the source line");
    }
    Ok(json!({ "line": line - 1, "character": requested_units }))
}

fn source_offset(source: &str, position: &Value) -> anyhow::Result<usize> {
    let line = position.get("line").and_then(Value::as_u64).unwrap_or(0) as usize;
    let character = position
        .get("character")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let mut offset = 0;
    for (index, line_text) in source.split_inclusive('\n').enumerate() {
        if index == line {
            let mut units = 0;
            for (byte, ch) in line_text.char_indices() {
                if units >= character {
                    return Ok(offset + byte);
                }
                units += ch.len_utf16();
            }
            let content = line_text.trim_end_matches(['\r', '\n']);
            let content_units = content.encode_utf16().count();
            if character == content_units {
                return Ok(offset + content.len());
            }
            bail!("LSP edit position is outside the source line");
        }
        offset += line_text.len();
    }
    if line == source.split('\n').count().saturating_sub(1) && character == 0 {
        return Ok(source.len());
    }
    bail!("LSP edit line is outside the source file")
}

fn identifier_at_lsp_position(source: &str, position: &Value) -> anyhow::Result<String> {
    let offset = source_offset(source, position)?;
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    let column = offset - line_start;
    let tokens = source
        .parse::<proc_macro2::TokenStream>()
        .map_err(|error| anyhow::anyhow!("tokenizing Rust source at rename position: {error}"))?;
    identifier_at_token_position(tokens, line, column)
        .ok_or_else(|| anyhow!("could not identify the Rust symbol at the rename position"))
}

fn identifier_at_token_position(
    tokens: proc_macro2::TokenStream,
    line: usize,
    column: usize,
) -> Option<String> {
    tokens.into_iter().find_map(|token| match token {
        proc_macro2::TokenTree::Ident(identifier) => {
            let span = identifier.span();
            let start = span.start();
            let end = span.end();
            (start.line == line
                && end.line == line
                && start.column <= column
                && column <= end.column)
                .then(|| normalized_ident_name(&identifier.to_string()))
        }
        proc_macro2::TokenTree::Group(group) => {
            identifier_at_token_position(group.stream(), line, column)
        }
        _ => None,
    })
}

fn normalized_ident_name(identifier: &str) -> String {
    identifier
        .strip_prefix("r#")
        .unwrap_or(identifier)
        .to_owned()
}

async fn find_unedited_macro_references(
    root: &Path,
    preview_files: &[PreviewFile],
    old_name: Option<&str>,
) -> MacroReferenceReport {
    let mut report = MacroReferenceReport {
        symbol: old_name.map(str::to_owned),
        candidates: Vec::new(),
        scanned_files: 0,
        scan_complete: old_name.is_some(),
        truncated: false,
        issue_count: 0,
    };
    let Some(old_name) = old_name else {
        report.issue_count = 1;
        return report;
    };

    let root = match tokio::fs::canonicalize(root).await {
        Ok(root) => root,
        Err(_) => {
            report.scan_complete = false;
            report.issue_count = 1;
            return report;
        }
    };
    let edited_sources = preview_files
        .iter()
        .map(|file| (file.path.clone(), file.after.as_slice()))
        .collect::<HashMap<_, _>>();
    let mut directories = vec![root.clone()];
    let mut source_files = Vec::new();

    while let Some(directory) = directories.pop() {
        let mut entries = match tokio::fs::read_dir(&directory).await {
            Ok(entries) => entries,
            Err(_) => {
                report.scan_complete = false;
                report.issue_count += 1;
                continue;
            }
        };
        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(_) => {
                    report.scan_complete = false;
                    report.issue_count += 1;
                    break;
                }
            };
            let path = entry.path();
            let file_type = match entry.file_type().await {
                Ok(file_type) => file_type,
                Err(_) => {
                    report.scan_complete = false;
                    report.issue_count += 1;
                    continue;
                }
            };
            if file_type.is_dir() {
                let name = entry.file_name();
                if matches!(
                    name.to_str(),
                    Some(".git" | ".codegraph" | ".aimer" | "target")
                ) {
                    continue;
                }
                directories.push(path);
            } else if file_type.is_file()
                && path.extension().and_then(std::ffi::OsStr::to_str) == Some("rs")
            {
                source_files.push(path);
            }
        }
    }
    source_files.sort();

    for path in source_files {
        let source = if let Some(source) = edited_sources.get(&path) {
            match std::str::from_utf8(source) {
                Ok(source) => source.to_owned(),
                Err(_) => {
                    report.scan_complete = false;
                    report.issue_count += 1;
                    continue;
                }
            }
        } else {
            match tokio::fs::read_to_string(&path).await {
                Ok(source) => source,
                Err(_) => {
                    report.scan_complete = false;
                    report.issue_count += 1;
                    continue;
                }
            }
        };
        let syntax = match syn::parse_file(&source) {
            Ok(syntax) => syntax,
            Err(_) => {
                report.scan_complete = false;
                report.issue_count += 1;
                continue;
            }
        };
        report.scanned_files += 1;
        let relative = path
            .strip_prefix(&root)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().into_owned());
        let lines = source.lines().collect::<Vec<_>>();
        let mut visitor = MacroReferenceVisitor {
            old_name,
            file: &relative,
            lines: &lines,
            candidates: &mut report.candidates,
            truncated: &mut report.truncated,
        };
        visitor.visit_file(&syntax);
    }

    report
}

struct MacroReferenceVisitor<'a> {
    old_name: &'a str,
    file: &'a str,
    lines: &'a [&'a str],
    candidates: &'a mut Vec<MacroReferenceCandidate>,
    truncated: &'a mut bool,
}

impl<'ast> Visit<'ast> for MacroReferenceVisitor<'_> {
    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if item.mac.path.is_ident("macro_rules") {
            let macro_name = item
                .ident
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "<unnamed>".to_owned());
            collect_macro_identifiers(
                item.mac.tokens.clone(),
                self.old_name,
                self.file,
                &macro_name,
                self.lines,
                self.candidates,
                self.truncated,
            );
        }
        syn::visit::visit_item_macro(self, item);
    }
}

fn collect_macro_identifiers(
    tokens: proc_macro2::TokenStream,
    old_name: &str,
    file: &str,
    macro_name: &str,
    lines: &[&str],
    candidates: &mut Vec<MacroReferenceCandidate>,
    truncated: &mut bool,
) {
    for token in tokens {
        match token {
            proc_macro2::TokenTree::Ident(identifier)
                if normalized_ident_name(&identifier.to_string()) == old_name =>
            {
                if candidates.len() >= MAX_RESULTS {
                    *truncated = true;
                    continue;
                }
                let start = identifier.span().start();
                let Some(line_text) = start.line.checked_sub(1).and_then(|line| lines.get(line))
                else {
                    *truncated = true;
                    continue;
                };
                let byte_column = start.column.min(line_text.len());
                let column = line_text[..byte_column].encode_utf16().count() + 1;
                candidates.push(MacroReferenceCandidate {
                    file: file.to_owned(),
                    line: start.line,
                    column,
                    macro_name: macro_name.to_owned(),
                    text: line_text.trim().to_owned(),
                });
            }
            proc_macro2::TokenTree::Group(group) => collect_macro_identifiers(
                group.stream(),
                old_name,
                file,
                macro_name,
                lines,
                candidates,
                truncated,
            ),
            _ => {}
        }
    }
}

fn macro_reference_report_json(report: &MacroReferenceReport) -> Value {
    json!({
        "symbol": report.symbol,
        "candidate_edits_added": false,
        "scan_complete": report.scan_complete,
        "scanned_files": report.scanned_files,
        "issue_count": report.issue_count,
        "truncated": report.truncated,
        "review_note": "These are possible old-name tokens remaining in macro_rules! bodies. They are unverified and were not added to the rename edit.",
        "items": report.candidates.iter().map(|candidate| json!({
            "file": candidate.file,
            "line": candidate.line,
            "column": candidate.column,
            "macro_name": candidate.macro_name,
            "identifier": report.symbol,
            "verified_reference": false,
            "text": candidate.text
        })).collect::<Vec<_>>()
    })
}

async fn workspace_edit_files(root: &Path, edit: &Value) -> anyhow::Result<Vec<PreviewFile>> {
    let root = root
        .canonicalize()
        .context("resolving project root for workspace edit")?;
    let mut grouped: HashMap<PathBuf, Vec<Value>> = HashMap::new();
    if let Some(changes) = edit.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            let path = reqwest::Url::parse(uri)
                .ok()
                .and_then(|url| url.to_file_path().ok())
                .ok_or_else(|| anyhow!("rust-analyzer returned a non-file edit URI"))?;
            grouped
                .entry(path)
                .or_default()
                .extend(edits.as_array().cloned().unwrap_or_default());
        }
    }
    if let Some(changes) = edit.get("documentChanges").and_then(Value::as_array) {
        for change in changes {
            let Some(document) = change.get("textDocument") else {
                bail!("workspace edit contains a file operation that Aimer cannot preview");
            };
            let uri = document
                .get("uri")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("workspace edit omitted document URI"))?;
            let path = reqwest::Url::parse(uri)
                .ok()
                .and_then(|url| url.to_file_path().ok())
                .ok_or_else(|| anyhow!("rust-analyzer returned a non-file edit URI"))?;
            grouped.entry(path).or_default().extend(
                change
                    .get("edits")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
        }
    }

    let mut files = Vec::new();
    for (path, edits) in grouped {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("resolving edit file '{}' ", path.display()))?;
        if !canonical.starts_with(&root) {
            bail!("workspace edit targets a file outside the selected project");
        }
        let before = tokio::fs::read(&canonical).await?;
        let mut text = String::from_utf8(before.clone())
            .context("rust-analyzer edit targeted a non-UTF-8 file")?;
        let mut indexed = Vec::new();
        for edit in edits {
            let range = edit
                .get("range")
                .ok_or_else(|| anyhow!("text edit omitted range"))?;
            let start = source_offset(&text, range.get("start").unwrap_or(&Value::Null))?;
            let end = source_offset(&text, range.get("end").unwrap_or(&Value::Null))?;
            if start > end
                || end > text.len()
                || !text.is_char_boundary(start)
                || !text.is_char_boundary(end)
            {
                bail!("rust-analyzer returned an invalid text edit range");
            }
            indexed.push((
                start,
                end,
                edit.get("newText")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            ));
        }
        indexed.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
        for (start, end, replacement) in indexed {
            text.replace_range(start..end, &replacement);
        }
        let relative = canonical
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        let after = text.into_bytes();
        let (_, diff_truncated) = unified_diff_with_flag(&before, &after);
        files.push(PreviewFile {
            path: canonical,
            relative,
            before,
            after,
            diff_truncated,
        });
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(files)
}

fn unified_diff(before: &[u8], after: &[u8]) -> String {
    unified_diff_with_flag(before, after).0
}

fn unified_diff_with_flag(before: &[u8], after: &[u8]) -> (String, bool) {
    const MAX_DIFF_BYTES: usize = 24 * 1024;
    const MAX_DIFF_LINES: usize = 300;
    let before = String::from_utf8_lossy(before);
    let after = String::from_utf8_lossy(after);
    let old = before.lines().collect::<Vec<_>>();
    let new = after.lines().collect::<Vec<_>>();
    let mut prefix = 0;
    while prefix < old.len().min(new.len()) && old[prefix] == new[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix
        < old
            .len()
            .saturating_sub(prefix)
            .min(new.len().saturating_sub(prefix))
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let old_changed = &old[prefix..old.len().saturating_sub(suffix)];
    let new_changed = &new[prefix..new.len().saturating_sub(suffix)];
    let mut lines = vec![format!("@@ line {} @@", prefix + 1)];
    lines.extend(old_changed.iter().map(|line| format!("-{line}")));
    lines.extend(new_changed.iter().map(|line| format!("+{line}")));
    let mut diff = lines.join("\n");
    let truncated = diff.len() > MAX_DIFF_BYTES || lines.len() > MAX_DIFF_LINES;
    if diff.len() > MAX_DIFF_BYTES {
        diff.truncate(MAX_DIFF_BYTES);
    }
    if lines.len() > MAX_DIFF_LINES {
        diff = lines
            .into_iter()
            .take(MAX_DIFF_LINES)
            .collect::<Vec<_>>()
            .join("\n");
    }
    (diff, truncated)
}

async fn replace_file_atomically(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("file has no parent directory"))?;
    let id = Uuid::new_v4();
    let temporary = parent.join(format!(".aimer-edit-{id}.tmp"));
    let backup = parent.join(format!(".aimer-edit-{id}.bak"));
    let permissions = tokio::fs::metadata(path).await?.permissions();
    tokio::fs::write(&temporary, bytes).await?;
    tokio::fs::set_permissions(&temporary, permissions).await?;
    if let Err(error) = tokio::fs::rename(path, &backup).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(error.into());
    }
    if let Err(error) = tokio::fs::rename(&temporary, path).await {
        let _ = tokio::fs::rename(&backup, path).await;
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(error.into());
    }
    let _ = tokio::fs::remove_file(&backup).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        RustAnalyzerBridge, flycheck_result_json, normalize_result, position_from_source,
        simple_glob, source_offset, unified_diff, workspace_edit_files,
    };
    use serde_json::json;
    use std::fs;

    #[test]
    fn position_conversion_uses_one_based_utf16_columns() {
        assert_eq!(
            position_from_source("a😀b\n", 1, 4).unwrap(),
            json!({"line": 0, "character": 3})
        );
        assert!(position_from_source("a😀b\n", 1, 3).is_err());
    }

    #[test]
    fn edit_offsets_handle_utf16_and_crlf_lines() {
        let source = "a😀b\r\nnext\r\n";
        assert_eq!(
            source_offset(source, &json!({"line": 0, "character": 3})).unwrap(),
            5
        );
        assert_eq!(
            source_offset(source, &json!({"line": 1, "character": 0})).unwrap(),
            8
        );
    }

    #[test]
    fn project_globs_support_recursive_directories() {
        assert!(simple_glob("src/**/*.rs", "src/main.rs"));
        assert!(simple_glob("src/**/*.rs", "src/core/lib.rs"));
        assert!(!simple_glob("src/*.rs", "src/core/lib.rs"));
    }

    #[test]
    fn diff_contains_removed_and_added_lines() {
        let diff = unified_diff(b"fn old() {}\n", b"fn new() {}\n");
        assert!(diff.contains("-fn old() {}"));
        assert!(diff.contains("+fn new() {}"));
    }

    #[test]
    fn normalized_lsp_symbols_are_compact_and_serialize_stably() {
        let temporary = tempfile::tempdir().unwrap();
        let input = json!({
            "deprecated": false,
            "tags": [],
            "kind": 12,
            "name": "read_config",
            "range": {
                "start": { "line": 4, "character": 0 },
                "end": { "line": 6, "character": 1 }
            },
            "selectionRange": {
                "start": { "line": 5, "character": 7 },
                "end": { "line": 5, "character": 18 }
            }
        });
        let reordered_input: serde_json::Value = serde_json::from_str(
            r#"{
                "selectionRange":{"end":{"character":18,"line":5},"start":{"character":7,"line":5}},
                "range":{"end":{"character":1,"line":6},"start":{"character":0,"line":4}},
                "name":"read_config",
                "kind":12,
                "tags":[],
                "deprecated":false
            }"#,
        )
        .unwrap();

        let normalized = normalize_result(temporary.path(), &input);
        let repeated = normalize_result(temporary.path(), &reordered_input);
        let serialized = serde_json::to_string(&normalized).unwrap();
        let repeated_serialized = serde_json::to_string(&repeated).unwrap();

        assert_eq!(normalized, repeated);
        assert_eq!(serialized, repeated_serialized);
        assert_eq!(
            serialized,
            r#"{"kind":"function","name":"read_config","range":{"end":{"column":2,"line":7},"start":{"column":1,"line":5}},"selection_range":{"end":{"column":19,"line":6},"start":{"column":8,"line":6}}}"#
        );
    }

    #[test]
    fn nested_lsp_result_truncation_is_reported() {
        let temporary = tempfile::tempdir().unwrap();
        let input = json!({
            "children": (0..=200)
                .map(|index| json!({ "name": format!("child_{index}") }))
                .collect::<Vec<_>>()
        });

        let normalized = normalize_result(temporary.path(), &input);

        assert_eq!(normalized["children"].as_array().unwrap().len(), 200);
        assert_eq!(normalized["children_truncated"], true);
    }

    #[test]
    fn clippy_result_reports_diagnostics_without_claiming_a_cargo_exit_code() {
        let result = flycheck_result_json(json!({
            "diagnostics": [
                { "severity": "error", "source": "rustc", "message": "mismatched types" },
                { "severity": "warning", "source": "rustc", "message": "unused variable" }
            ],
            "truncated": false
        }));

        assert_eq!(result["operation"], "cargo_clippy");
        assert_eq!(result["command"], "clippy");
        assert_eq!(result["warnings_denied"], true);
        assert_eq!(result["status"], "completed_with_errors");
        assert_eq!(result["cargo_exit_status"], "not_reported_by_rust_analyzer");
        assert_eq!(result["diagnostic_summary"]["error_count"], 1);
        assert_eq!(result["diagnostic_summary"]["warning_count"], 1);
        assert_eq!(result["diagnostics"].as_array().unwrap().len(), 2);

        let no_errors = flycheck_result_json(json!({
            "diagnostics": [],
            "truncated": false
        }));
        assert_eq!(no_errors["status"], "completed");
        assert_eq!(no_errors["diagnostic_summary"]["error_count"], 0);
    }

    #[tokio::test]
    async fn workspace_edit_preview_is_bounded_to_project_files() {
        let temporary = tempfile::tempdir().unwrap();
        let file = temporary.path().join("lib.rs");
        fs::write(&file, "fn old() {}\n").unwrap();
        let uri = reqwest::Url::from_file_path(&file).unwrap().to_string();
        let edit = json!({
            "changes": { uri: [{
                "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                "newText": "new"
            }] }
        });
        let preview = workspace_edit_files(temporary.path(), &edit).await.unwrap();
        assert_eq!(preview.len(), 1);
        assert_eq!(preview[0].before, b"fn old() {}\n");
        assert_eq!(preview[0].after, b"fn new() {}\n");
        assert_eq!(fs::read(&file).unwrap(), b"fn old() {}\n");
    }

    #[tokio::test]
    async fn failed_manifest_validation_prevents_apply_and_keeps_file_unchanged() {
        let temporary = tempfile::tempdir().unwrap();
        let manifest = temporary.path().join("Cargo.toml");
        let before = b"[package]\nname = \"sample\"\nversion = \"0.1.0\"\n";
        let after = b"[package]\nname = \"sample\"\nversion = \"invalid\"\n";
        fs::write(&manifest, before).unwrap();
        let bridge = RustAnalyzerBridge::new(
            temporary.path().to_path_buf(),
            aimer_hot_reload::config::RustAnalyzerConfig::default(),
        );
        let preview = bridge
            .prepare_manifest_content_preview(
                std::path::Path::new("Cargo.toml"),
                before,
                after,
                json!({ "status": "failed", "message": "candidate manifest is invalid" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(preview["applicable"], false);
        let preview_id = preview["preview_id"].as_str().unwrap();
        assert!(bridge.apply_preview(preview_id).await.is_err());
        assert_eq!(fs::read(&manifest).unwrap(), before);
        assert!(bridge.apply_preview(preview_id).await.is_err());
    }
}
