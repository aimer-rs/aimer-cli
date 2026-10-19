use super::*;

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

    /// Load or reuse the embedded Rust analysis engine for the selected workspace.
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
            // Dropping the prior session releases its embedded analysis database and helper.
            *session = None;
            *self
                .project_root
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = requested_root.clone();
            self.actions.lock().await.clear();
            self.previews.lock().await.clear();
        }
        let stale_workspace = session
            .as_mut()
            .map(EmbeddedRustAnalyzer::poll_file_events)
            .transpose()?
            .unwrap_or(false);
        if stale_workspace {
            *session = None;
        }
        let started = session.is_none();
        self.ensure_session_started(&mut session, progress).await?;
        let analyzer = session.as_ref().expect("session initialized");
        Ok(json!({
            "status": "ready",
            "started": started,
            "workspace_ready": true,
            "health": "ok",
            "message": Value::Null,
            "engine": "embedded",
            "proc_macro_server": analyzer.proc_macro_server().map(|path| path.display().to_string()),
            "proc_macro_enabled": analyzer.proc_macro_server().is_some(),
            "process_id": std::process::id(),
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
        if session
            .as_mut()
            .map(EmbeddedRustAnalyzer::poll_file_events)
            .transpose()?
            .unwrap_or(false)
        {
            *session = None;
        }
        self.ensure_session_started(&mut session, progress).await?;
        session
            .as_ref()
            .expect("embedded engine initialized")
            .request(method, &params)
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
        if session
            .as_mut()
            .map(EmbeddedRustAnalyzer::poll_file_events)
            .transpose()?
            .unwrap_or(false)
        {
            *session = None;
        }
        self.ensure_session_started(&mut session, progress).await?;
        let update = session
            .as_mut()
            .expect("embedded engine initialized")
            .update_document(&absolute, &contents);
        if update.is_err() {
            *session = None;
            self.ensure_session_started(&mut session, progress).await?;
            session
                .as_mut()
                .expect("embedded engine initialized")
                .update_document(&absolute, &contents)?;
        }
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
        if session
            .as_mut()
            .map(EmbeddedRustAnalyzer::poll_file_events)
            .transpose()?
            .unwrap_or(false)
        {
            *session = None;
        }
        self.ensure_session_started(&mut session, progress).await?;
        let filter = file
            .map(|file| self.resolve_project_file(file, false))
            .transpose()?;
        session
            .as_ref()
            .expect("embedded engine initialized")
            .diagnostics(filter.as_deref())
    }

    pub async fn get_code_actions(
        &self,
        file: &Path,
        range: Value,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<Value> {
        let (uri, _) = self.sync_document(file, progress).await?;
        let result = self
            .request(
                "textDocument/codeAction",
                json!({
                    "textDocument": { "uri": uri },
                    "range": range,
                    "context": { "diagnostics": [] }
                }),
                progress,
            )
            .await?;
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
        let (uri, source) = self.sync_document(file, progress).await?;
        let absolute = self.resolve_project_file(file, false)?;
        let project_root = self.project_root();
        let rustfmt_config = absolute.ancestors().find_map(|directory| {
            ["rustfmt.toml", ".rustfmt.toml"]
                .into_iter()
                .map(|name| directory.join(name))
                .find(|path| path.is_file())
        });
        let is_range_formatting = range.is_some();
        let mut command = if is_range_formatting {
            let mut command = Command::new("rustup");
            command.args(["run", "nightly", "rustfmt"]);
            command
        } else {
            Command::new("rustfmt")
        };
        command
            .args([
                "--edition",
                &self.project_edition(&absolute),
                "--emit",
                "stdout",
            ])
            .current_dir(&project_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(config_path) = rustfmt_config {
            command.arg("--config-path").arg(config_path);
        }
        let temporary_input = if let Some(range) = range {
            let start_line = range
                .get("start")
                .and_then(|position| position.get("line"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let end_position = range.get("end").unwrap_or(&Value::Null);
            let end_line = end_position
                .get("line")
                .and_then(Value::as_u64)
                .unwrap_or(start_line);
            let end_column = end_position
                .get("character")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let first_line = start_line + 1;
            // LSP ends are exclusive; rustfmt's file-line ranges are inclusive.
            let last_line = if end_line > start_line && end_column == 0 {
                end_line
            } else {
                end_line + 1
            }
            .max(first_line);
            let temporary_input = tempfile::NamedTempFile::new()
                .context("creating a temporary Rust formatting input")?;
            std::fs::write(temporary_input.path(), source.as_bytes())
                .context("writing the temporary Rust formatting input")?;
            let file_lines =
                json!([{ "file": temporary_input.path(), "range": [first_line, last_line] }]);
            command.args([
                "--unstable-features",
                "--skip-children",
                "--file-lines",
                &file_lines.to_string(),
            ]);
            command.arg(temporary_input.path());
            Some(temporary_input)
        } else {
            command.stdin(Stdio::piped());
            None
        };
        let mut child = command.spawn().context(if is_range_formatting {
            "starting Nightly rustfmt for range formatting"
        } else {
            "starting rustfmt"
        })?;
        if temporary_input.is_none() {
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(source.as_bytes())
                    .await
                    .context("sending Rust source to rustfmt")?;
            }
        }
        let output = timeout(Duration::from_secs(30), child.wait_with_output())
            .await
            .context("rustfmt exceeded the 30 second timeout")?
            .context("waiting for rustfmt")?;
        if !output.status.success() {
            let details = String::from_utf8_lossy(&output.stderr);
            bail!("rustfmt failed: {}", details.trim());
        }
        let formatted =
            String::from_utf8(output.stdout).context("rustfmt returned non-UTF-8 output")?;
        let formatted = if let Some(temporary_input) = temporary_input.as_ref() {
            let (header, formatted_source) = formatted
                .split_once('\n')
                .ok_or_else(|| anyhow!("rustfmt output did not include its file header"))?;
            let reported_path = header
                .trim_end_matches('\r')
                .strip_suffix(':')
                .ok_or_else(|| anyhow!("rustfmt output did not include its file header"))?;
            anyhow::ensure!(
                Path::new(reported_path).file_name() == temporary_input.path().file_name(),
                "rustfmt returned output for a different file"
            );
            formatted_source
                .strip_prefix("\r\n")
                .or_else(|| formatted_source.strip_prefix('\n'))
                .unwrap_or(formatted_source)
                .to_owned()
        } else {
            formatted
        };
        drop(temporary_input);
        let edit = json!({ "changes": { uri: [{
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": source_end_position(&source)
            },
            "newText": formatted
        }] } });
        self.create_preview(edit).await
    }
}
