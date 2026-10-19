#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(feature = "legacy-lsp")]
struct RustAnalyzerStatus {
    health: String,
    quiescent: bool,
    message: Option<String>,
}

#[derive(Debug)]
#[cfg(feature = "legacy-lsp")]
struct LspServerError {
    method: String,
    details: String,
}

#[cfg(feature = "legacy-lsp")]
impl std::fmt::Display for LspServerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "rust-analyzer request {} failed: {}",
            self.method, self.details
        )
    }
}

#[cfg(feature = "legacy-lsp")]
impl std::error::Error for LspServerError {}

use super::*;

#[cfg(feature = "legacy-lsp")]
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

#[cfg(feature = "legacy-lsp")]
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

#[cfg(feature = "legacy-lsp")]
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
            let Some(file) = uri_to_relative(root, uri) else {
                continue;
            };
            for diagnostic in diagnostics {
                let mut result = json!({
                    "file": file,
                    "range": normalize_range(diagnostic.get("range")),
                    "severity": diagnostic
                        .get("severity")
                        .and_then(Value::as_u64)
                        .map(diagnostic_severity_name)
                        .map(|value| Value::String(value.to_owned()))
                        .unwrap_or(Value::Null),
                });
                for key in ["code", "source", "message"] {
                    if let Some(value) = diagnostic.get(key).filter(|value| !value.is_null()) {
                        result[key] = value.clone();
                    }
                }
                if let Some(related) = diagnostic
                    .get("relatedInformation")
                    .filter(|value| value.as_array().is_some_and(|items| !items.is_empty()))
                {
                    result["related_information"] = normalize_lsp_value(root, related);
                }
                all.push(result);
            }
        }
        all.sort_by(|left, right| {
            let left_start = &left["range"]["start"];
            let right_start = &right["range"]["start"];
            left["file"]
                .as_str()
                .cmp(&right["file"].as_str())
                .then_with(|| {
                    left_start["line"]
                        .as_u64()
                        .cmp(&right_start["line"].as_u64())
                })
                .then_with(|| {
                    left_start["column"]
                        .as_u64()
                        .cmp(&right_start["column"].as_u64())
                })
                .then_with(|| left["severity"].as_str().cmp(&right["severity"].as_str()))
                .then_with(|| left["code"].as_str().cmp(&right["code"].as_str()))
                .then_with(|| left["message"].as_str().cmp(&right["message"].as_str()))
        });
        let truncated = all.len() > MAX_RESULTS;
        all.truncate(MAX_RESULTS);
        let mut result = json!({ "diagnostics": all });
        if truncated {
            result["truncated"] = Value::Bool(true);
        }
        result
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

#[cfg(feature = "legacy-lsp")]
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

#[cfg(feature = "legacy-lsp")]
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

#[cfg(feature = "legacy-lsp")]
fn format_status(status: &RustAnalyzerStatus) -> String {
    format!(
        "health={}, quiescent={}, message={}",
        status.health,
        status.quiescent,
        status.message.as_deref().unwrap_or("none")
    )
}

#[cfg(feature = "legacy-lsp")]
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
