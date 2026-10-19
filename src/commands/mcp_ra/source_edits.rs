use super::macro_references::MacroReferenceReport;
use super::*;

#[derive(Debug)]
pub(super) struct PreviewFile {
    pub(super) path: PathBuf,
    pub(super) relative: String,
    pub(super) before: Option<Vec<u8>>,
    pub(super) after: Option<Vec<u8>>,
    pub(super) before_permissions: Option<std::fs::Permissions>,
    pub(super) diff_truncated: bool,
}

pub(super) enum SourceEdit {
    Edit(Vec<TextReplacement>),
    Create(String),
    Delete,
}

pub(super) fn preview_file_action(file: &PreviewFile) -> &'static str {
    match (&file.before, &file.after) {
        (None, Some(_)) => "create",
        (Some(_), None) => "delete",
        (Some(_), Some(_)) => "edit",
        (None, None) => "unchanged",
    }
}

#[derive(Debug)]
pub(super) struct EditPreview {
    pub(super) files: Vec<PreviewFile>,
    pub(super) macro_reference_candidates: Option<MacroReferenceReport>,
    pub(super) manifest_validation: Option<Value>,
    pub(super) source_validation: Option<Value>,
    pub(super) applicable: bool,
    pub(super) created_at: Instant,
}

pub(super) async fn workspace_edit_files(
    root: &Path,
    edit: &Value,
) -> anyhow::Result<Vec<PreviewFile>> {
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
            before: Some(before),
            after: Some(after),
            before_permissions: Some(tokio::fs::metadata(&path).await?.permissions()),
            diff_truncated,
        });
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(files)
}

pub(super) fn unified_diff(before: &[u8], after: &[u8]) -> String {
    unified_diff_with_flag(before, after).0
}

pub(super) fn unified_diff_with_flag(before: &[u8], after: &[u8]) -> (String, bool) {
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

pub(super) async fn replace_file_atomically(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
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

pub(super) async fn create_file_atomically(
    path: &Path,
    bytes: &[u8],
    permissions: Option<&std::fs::Permissions>,
) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("file has no parent directory"))?;
    let temporary = parent.join(format!(".aimer-edit-{}.tmp", Uuid::new_v4()));
    tokio::fs::write(&temporary, bytes)
        .await
        .context("writing a temporary file for the edit")?;
    if let Some(permissions) = permissions {
        if let Err(error) = tokio::fs::set_permissions(&temporary, permissions.clone()).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error).context("restoring file permissions during edit rollback");
        }
    }
    if let Err(error) = tokio::fs::hard_link(&temporary, path).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(error).context("creating file atomically (target may already exist)");
    }
    let _ = tokio::fs::remove_file(&temporary).await;
    Ok(())
}

pub(super) async fn delete_file_atomically(path: &Path) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("file has no parent directory"))?;
    let backup = parent.join(format!(".aimer-edit-{}.bak", Uuid::new_v4()));
    tokio::fs::rename(path, &backup)
        .await
        .context("moving the file aside for atomic deletion")?;
    if let Err(error) = tokio::fs::remove_file(&backup).await {
        if let Err(restore_error) = tokio::fs::rename(&backup, path).await {
            return Err(error).context(format!(
                "could not remove the deletion backup and could not restore it: {restore_error}"
            ));
        }
        return Err(error).context("could not remove the deletion backup; original file restored");
    }
    Ok(())
}

pub(super) async fn apply_preview_file(file: &PreviewFile) -> anyhow::Result<()> {
    match (&file.before, &file.after) {
        (Some(_), Some(after)) => replace_file_atomically(&file.path, after).await,
        (None, Some(after)) => create_file_atomically(&file.path, after, None).await,
        (Some(_), None) => delete_file_atomically(&file.path).await,
        (None, None) => bail!("edit preview contains an empty file operation"),
    }
}

pub(super) async fn path_entry_exists(path: &Path) -> anyhow::Result<bool> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(super) async fn restore_preview_file(file: &PreviewFile) -> anyhow::Result<()> {
    match &file.before {
        Some(before) => {
            if tokio::fs::try_exists(&file.path).await? {
                replace_file_atomically(&file.path, before).await
            } else {
                create_file_atomically(&file.path, before, file.before_permissions.as_ref()).await
            }
        }
        None => match tokio::fs::remove_file(&file.path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        },
    }
}
