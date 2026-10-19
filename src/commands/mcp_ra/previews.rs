use super::*;

impl RustAnalyzerBridge {
    pub async fn apply_preview(&self, preview_id: &str) -> anyhow::Result<Value> {
        let preview = {
            let mut previews = self.previews.lock().await;
            let existing = previews
                .get(preview_id)
                .ok_or_else(|| anyhow!("unknown, expired, or already applied edit preview"))?;
            if !existing.applicable {
                bail!("edit preview cannot be applied because its validation failed");
            }
            previews
                .remove(preview_id)
                .expect("preview was just checked")
        };
        if preview.created_at.elapsed() > Duration::from_secs(300) {
            bail!("edit preview expired; prepare a new preview before applying");
        }
        for file in &preview.files {
            let current_path = self.resolve_project_file(&file.path, true)?;
            if current_path != file.path {
                bail!(
                    "edit preview is stale because '{}' now resolves to a different path",
                    file.relative
                );
            }
            match &file.before {
                Some(before) => {
                    let current = tokio::fs::read(&file.path).await.with_context(|| {
                        format!("reading '{}' before applying edit", file.relative)
                    })?;
                    if &current != before {
                        bail!("edit preview is stale because '{}' changed", file.relative);
                    }
                }
                None => {
                    if path_entry_exists(&file.path).await? {
                        bail!(
                            "edit preview is stale because '{}' now exists",
                            file.relative
                        );
                    }
                }
            }
        }

        let mut applied: Vec<&PreviewFile> = Vec::new();
        for file in &preview.files {
            let result = apply_preview_file(file).await;
            if let Err(error) = result {
                let mut rollback_errors = Vec::new();
                for previous in applied.iter().rev() {
                    if let Err(rollback_error) = restore_preview_file(previous).await {
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
        let requires_workspace_reload = preview.files.iter().any(|file| {
            file.path.file_name().is_some_and(|name| name == "build.rs")
                || (file
                    .path
                    .extension()
                    .is_some_and(|extension| extension == "rs")
                    && (file.before.is_none() || file.after.is_none()))
        });
        let mut workspace_reload = None;
        let mut session = self.session.lock().await;
        if contains_manifest || requires_workspace_reload {
            *session = None;
            workspace_reload =
                Some("embedded workspace will reload on the next Rust request".to_owned());
        } else if let Some(engine) = session.as_mut() {
            for file in &preview.files {
                if file
                    .path
                    .extension()
                    .is_some_and(|extension| extension == "rs")
                {
                    if let Some(after) = file.after.as_deref()
                        && let Ok(source) = std::str::from_utf8(after)
                    {
                        if engine.update_document(&file.path, source).is_err() {
                            *session = None;
                            workspace_reload = Some(
                                "embedded workspace will reload on the next Rust request"
                                    .to_owned(),
                            );
                            break;
                        }
                    }
                }
            }
        }
        let mut result = json!({
            "applied": true,
            "files": preview.files.iter().map(|file| file.relative.clone()).collect::<Vec<_>>()
        });
        result["file_changes"] = json!(
            preview
                .files
                .iter()
                .map(|file| json!({
                    "file": file.relative,
                    "action": preview_file_action(file)
                }))
                .collect::<Vec<_>>()
        );
        if let Some(validation) = &preview.manifest_validation {
            result["manifest_validation"] = validation.clone();
        }
        if let Some(validation) = &preview.source_validation {
            result["source_validation"] = validation.clone();
        }
        if let Some(reload) = workspace_reload {
            result["workspace_reload"] = json!(reload);
        }
        if let Some(report) = &preview.macro_reference_candidates {
            result["macro_reference_candidates"] = macro_reference_report_json(report);
        }
        Ok(result)
    }

    pub(super) async fn ensure_session_started(
        &self,
        session: &mut Option<EmbeddedRustAnalyzer>,
        progress: &McpProgressReporter,
    ) -> anyhow::Result<()> {
        if session.is_none() {
            progress
                .report("Loading the embedded rust-analyzer engine; this tool waits for workspace loading to finish.")
                .await;
            let root = self.project_root();
            let config = self.config.clone();
            let (sender, mut receiver) = mpsc::unbounded_channel::<String>();
            let task = tokio::task::spawn_blocking(move || {
                EmbeddedRustAnalyzer::load(root, &config, &|message| {
                    let _ = sender.send(message);
                })
            });
            tokio::pin!(task);
            loop {
                tokio::select! {
                    result = &mut task => {
                        while let Ok(message) = receiver.try_recv() {
                            progress.report(message).await;
                        }
                        *session = Some(result.context("embedded Rust analysis task failed")?.context("loading embedded rust-analyzer")?);
                        break;
                    }
                    message = receiver.recv() => if let Some(message) = message {
                        progress.report(message).await;
                    },
                }
            }
        }
        Ok(())
    }

    pub(super) async fn create_preview(&self, workspace_edit: Value) -> anyhow::Result<Value> {
        let project_root = self.project_root();
        let files = workspace_edit_files(&project_root, &workspace_edit).await?;
        self.finish_preview(files, None).await
    }

    /// Prepare exact, project-confined text edits and file operations for review.
    pub async fn prepare_source_edit_preview(
        &self,
        changes: Vec<SourceFileChange>,
    ) -> anyhow::Result<Value> {
        const MAX_FILES: usize = 20;
        const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
        const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;

        anyhow::ensure!(!changes.is_empty(), "provide at least one file change");
        anyhow::ensure!(
            changes.len() <= MAX_FILES,
            "a preview can contain at most {MAX_FILES} file changes"
        );
        let project_root = self
            .project_root()
            .canonicalize()
            .context("resolving selected Rust project root")?;
        let mut files = Vec::with_capacity(changes.len());
        let mut seen_paths = Vec::with_capacity(changes.len());
        let mut batch_bytes = 0usize;
        let mut replacement_bytes = 0usize;

        for change in changes {
            let (relative_input, operation) = match change {
                SourceFileChange::Edit { file, replacements } => {
                    (file, SourceEdit::Edit(replacements))
                }
                SourceFileChange::Create { file, content } => (file, SourceEdit::Create(content)),
                SourceFileChange::Delete { file } => (file, SourceEdit::Delete),
            };
            let relative_path = Path::new(&relative_input);
            anyhow::ensure!(
                !relative_path.is_absolute(),
                "file paths must be project-relative"
            );
            anyhow::ensure!(
                !relative_path
                    .file_name()
                    .is_some_and(|name| name == "Cargo.toml"),
                "use preview_manifest_edit for Cargo.toml changes so Cargo validation runs"
            );
            let allow_missing = matches!(&operation, SourceEdit::Create(_));
            let path = self.resolve_project_file(relative_path, allow_missing)?;
            anyhow::ensure!(
                !path.file_name().is_some_and(|name| name == "Cargo.toml"),
                "use preview_manifest_edit for Cargo.toml changes so Cargo validation runs"
            );
            anyhow::ensure!(
                !seen_paths.contains(&path),
                "each file can appear only once in a preview"
            );
            seen_paths.push(path.clone());
            let before_permissions = if allow_missing {
                anyhow::ensure!(
                    !path_entry_exists(&path).await?,
                    "cannot create '{}': it already exists",
                    relative_input
                );
                None
            } else {
                let metadata = tokio::fs::metadata(&path).await.with_context(|| {
                    format!("reading project file '{}' for preview", relative_input)
                })?;
                anyhow::ensure!(
                    metadata.is_file(),
                    "'{}' is not a regular file",
                    relative_input
                );
                anyhow::ensure!(
                    metadata.len() <= MAX_FILE_BYTES as u64,
                    "'{}' exceeds the 2 MiB preview limit",
                    relative_input
                );
                Some(metadata.permissions())
            };

            let (before, after) = match operation {
                SourceEdit::Edit(replacements) => {
                    anyhow::ensure!(
                        !replacements.is_empty() && replacements.len() <= 100,
                        "an edit must contain 1 to 100 text replacements"
                    );
                    for replacement in &replacements {
                        replacement_bytes = replacement_bytes
                            .saturating_add(replacement.old_text.len())
                            .saturating_add(replacement.new_text.len());
                        anyhow::ensure!(
                            replacement_bytes <= MAX_BATCH_BYTES,
                            "edit anchors and replacements cannot exceed 8 MiB"
                        );
                    }
                    let before = tokio::fs::read(&path)
                        .await
                        .with_context(|| format!("reading '{}' for preview", relative_input))?;
                    let source = String::from_utf8(before.clone())
                        .with_context(|| format!("'{}' is not UTF-8 text", relative_input))?;
                    let mut indexed = Vec::with_capacity(replacements.len());
                    for replacement in replacements {
                        anyhow::ensure!(
                            !replacement.old_text.is_empty(),
                            "old_text must not be empty in '{}'",
                            relative_input
                        );
                        let start = source.find(&replacement.old_text).ok_or_else(|| {
                            anyhow!("old_text was not found in '{}'", relative_input)
                        })?;
                        let next_character = source[start..]
                            .chars()
                            .next()
                            .expect("a non-empty old_text matched a source character")
                            .len_utf8();
                        anyhow::ensure!(
                            !source[start + next_character..].contains(&replacement.old_text),
                            "old_text must match exactly once in '{}'",
                            relative_input
                        );
                        anyhow::ensure!(
                            replacement.old_text != replacement.new_text,
                            "old_text and new_text are identical in '{}'",
                            relative_input
                        );
                        indexed.push((
                            start,
                            start + replacement.old_text.len(),
                            replacement.new_text,
                        ));
                    }
                    indexed.sort_by_key(|(start, end, _)| (*start, *end));
                    anyhow::ensure!(
                        indexed.windows(2).all(|pair| pair[0].1 <= pair[1].0),
                        "text replacements overlap in '{}'",
                        relative_input
                    );
                    let mut after = source;
                    for (start, end, replacement) in indexed.into_iter().rev() {
                        after.replace_range(start..end, &replacement);
                    }
                    (Some(before), Some(after.into_bytes()))
                }
                SourceEdit::Create(content) => (None, Some(content.into_bytes())),
                SourceEdit::Delete => {
                    let before = tokio::fs::read(&path)
                        .await
                        .with_context(|| format!("reading '{}' for preview", relative_input))?;
                    (Some(before), None)
                }
            };
            let before_size = before.as_ref().map_or(0, Vec::len);
            let after_size = after.as_ref().map_or(0, Vec::len);
            anyhow::ensure!(
                before_size <= MAX_FILE_BYTES && after_size <= MAX_FILE_BYTES,
                "'{}' exceeds the 2 MiB preview limit",
                relative_input
            );
            batch_bytes = batch_bytes.saturating_add(before_size.max(after_size));
            anyhow::ensure!(
                batch_bytes <= MAX_BATCH_BYTES,
                "a preview cannot exceed 8 MiB of changed file contents"
            );

            let relative = path
                .strip_prefix(&project_root)
                .context("edit path is outside the selected Rust project")?
                .to_string_lossy()
                .replace('\\', "/");
            let (_, diff_truncated) = unified_diff_with_flag(
                before.as_deref().unwrap_or_default(),
                after.as_deref().unwrap_or_default(),
            );
            files.push(PreviewFile {
                path,
                relative,
                before,
                after,
                before_permissions,
                diff_truncated,
            });
        }
        files.sort_by(|left, right| left.relative.cmp(&right.relative));

        let source_validation = json!({
            "rust_files": files.iter().filter_map(|file| {
                if !file.path.extension().is_some_and(|extension| extension == "rs") {
                    return None;
                }
                let Some(after) = file.after.as_deref() else {
                    return Some(json!({ "file": file.relative, "status": "deleted" }));
                };
                let status = match std::str::from_utf8(after) {
                    Ok(source) => match syn::parse_file(source) {
                        Ok(_) => json!({ "file": file.relative, "status": "valid" }),
                        Err(error) => json!({ "file": file.relative, "status": "syntax_error", "message": error.to_string() }),
                    },
                    Err(_) => json!({ "file": file.relative, "status": "invalid_utf8" }),
                };
                Some(status)
            }).collect::<Vec<_>>()
        });
        self.finish_preview_with_source_validation(files, source_validation)
            .await
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
        let before_permissions = tokio::fs::metadata(&path).await?.permissions();
        let relative = path
            .strip_prefix(&project_root)
            .context("manifest is outside the selected Rust project")?
            .to_string_lossy()
            .replace('\\', "/");
        let (diff, diff_truncated) = unified_diff_with_flag(before, after);
        let file = PreviewFile {
            path,
            relative,
            before: Some(before.to_vec()),
            after: Some(after.to_vec()),
            before_permissions: Some(before_permissions),
            diff_truncated,
        };
        self.finish_preview_with_validation(vec![file], validation, applicable)
            .await
            .map(|mut result| {
                result["files"][0]["diff"] = json!(diff);
                result
            })
    }

    pub(super) async fn create_rename_preview(
        &self,
        workspace_edit: Value,
        old_name: Option<&str>,
    ) -> anyhow::Result<Value> {
        let project_root = self.project_root();
        let files = workspace_edit_files(&project_root, &workspace_edit).await?;
        let report = find_unedited_macro_references(&project_root, &files, old_name).await;
        self.finish_preview(files, Some(report)).await
    }

    pub(super) async fn finish_preview(
        &self,
        files: Vec<PreviewFile>,
        macro_reference_candidates: Option<MacroReferenceReport>,
    ) -> anyhow::Result<Value> {
        self.finish_preview_inner(files, macro_reference_candidates, None, true, None)
            .await
    }

    pub(super) async fn finish_preview_with_validation(
        &self,
        files: Vec<PreviewFile>,
        validation: Value,
        applicable: bool,
    ) -> anyhow::Result<Value> {
        self.finish_preview_inner(files, None, Some(validation), applicable, None)
            .await
    }

    pub(super) async fn finish_preview_with_source_validation(
        &self,
        files: Vec<PreviewFile>,
        validation: Value,
    ) -> anyhow::Result<Value> {
        self.finish_preview_inner(files, None, None, true, Some(validation))
            .await
    }

    pub(super) async fn finish_preview_inner(
        &self,
        files: Vec<PreviewFile>,
        macro_reference_candidates: Option<MacroReferenceReport>,
        manifest_validation: Option<Value>,
        applicable: bool,
        source_validation: Option<Value>,
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
            if let Some(validation) = &source_validation {
                result["source_validation"] = validation.clone();
            }
            return Ok(result);
        }
        let preview_id = Uuid::new_v4().to_string();
        let file_views = files
            .iter()
            .map(|file| {
                json!({
                    "file": file.relative,
                    "action": preview_file_action(file),
                    "diff": unified_diff(
                        file.before.as_deref().unwrap_or_default(),
                        file.after.as_deref().unwrap_or_default()
                    ),
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
                source_validation: source_validation.clone(),
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
        if let Some(validation) = &source_validation {
            result["source_validation"] = validation.clone();
        }
        Ok(result)
    }

    pub(super) fn resolve_project_file(
        &self,
        file: &Path,
        allow_missing: bool,
    ) -> anyhow::Result<PathBuf> {
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

    pub(super) fn project_edition(&self, file: &Path) -> String {
        let root = self.project_root();
        let mut directory = file.parent();
        while let Some(current) = directory {
            if !current.starts_with(&root) {
                break;
            }
            let manifest = current.join("Cargo.toml");
            if let Ok(contents) = std::fs::read_to_string(manifest)
                && let Ok(value) = contents.parse::<toml::Value>()
            {
                if let Some(edition) = value
                    .get("package")
                    .and_then(|package| package.get("edition"))
                    .and_then(toml::Value::as_str)
                {
                    return edition.to_owned();
                }
                if let Some(edition) = value
                    .get("workspace")
                    .and_then(|workspace| workspace.get("package"))
                    .and_then(|package| package.get("edition"))
                    .and_then(toml::Value::as_str)
                {
                    return edition.to_owned();
                }
            }
            if current == root {
                break;
            }
            directory = current.parent();
        }
        "2015".to_owned()
    }
}
