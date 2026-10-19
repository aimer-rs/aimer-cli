use super::*;

impl EmbeddedRustAnalyzer {
    pub(super) fn position(
        &self,
        analysis: &Analysis,
        params: &Value,
    ) -> anyhow::Result<FilePosition> {
        let file_id = self.file_id_for_params(params)?;
        let source = self.file_text(analysis, file_id)?;
        let position = params.get("position").unwrap_or(&Value::Null);
        let offset = source_offset(&source, position)?;
        Ok(FilePosition {
            file_id,
            offset: TextSize::from(offset as u32),
        })
    }

    pub(super) fn file_id_for_params(&self, params: &Value) -> anyhow::Result<FileId> {
        let uri = params
            .pointer("/textDocument/uri")
            .or_else(|| params.pointer("/item/uri"))
            .or_else(|| params.pointer("/item/data/uri"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("request omitted textDocument URI"))?;
        self.file_id_for_path(&uri_to_path(uri)?)
    }

    pub(super) fn file_id_for_path(&self, path: &Path) -> anyhow::Result<FileId> {
        let path = self.resolve_path(path)?;
        let vfs_path = VfsPath::new_real_path(path.to_string_lossy().into_owned());
        match self.vfs.file_id(&vfs_path) {
            Some((file_id, FileExcluded::No)) => Ok(file_id),
            Some((_, FileExcluded::Yes)) => bail!(
                "Rust source '{}' is excluded from the workspace",
                path.display()
            ),
            None => bail!(
                "Rust source '{}' is not present in the loaded workspace",
                path.display()
            ),
        }
    }

    pub(super) fn file_text(&self, analysis: &Analysis, file_id: FileId) -> anyhow::Result<String> {
        analysis
            .file_text(file_id)
            .map(|text| text.to_string())
            .map_err(|error| {
                anyhow!("reading Rust source from the analysis database was cancelled: {error:?}")
            })
    }

    pub(super) fn file_uri(&self, file_id: FileId) -> anyhow::Result<String> {
        let path = self
            .vfs
            .file_path(file_id)
            .as_path()
            .ok_or_else(|| anyhow!("analysis target is not a filesystem path"))?;
        reqwest::Url::from_file_path(path.as_os_str())
            .map(|uri| uri.to_string())
            .map_err(|_| anyhow!("cannot convert analysis path '{}' to a file URI", path))
    }

    pub(super) fn relative_file(&self, file_id: FileId) -> String {
        let path = self.vfs.file_path(file_id).to_string();
        Path::new(&path)
            .strip_prefix(&self.root)
            .ok()
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .unwrap_or(path)
    }

    pub(super) fn navigation_location(
        &self,
        analysis: &Analysis,
        target: &NavigationTarget,
    ) -> anyhow::Result<(String, Value)> {
        let uri = self.file_uri(target.file_id)?;
        let source = self.file_text(analysis, target.file_id)?;
        Ok((uri, text_range_to_lsp(&source, target.full_range)))
    }

    pub(super) fn navigation_selection(
        &self,
        analysis: &Analysis,
        target: &NavigationTarget,
    ) -> anyhow::Result<(String, Value)> {
        let uri = self.file_uri(target.file_id)?;
        let source = self.file_text(analysis, target.file_id)?;
        Ok((
            uri,
            text_range_to_lsp(&source, target.focus_or_full_range()),
        ))
    }

    pub(super) fn resolve_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        let canonical = candidate
            .canonicalize()
            .with_context(|| format!("resolving '{}'", candidate.display()))?;
        if !canonical.starts_with(&self.root) {
            bail!("file path must stay inside the selected project");
        }
        Ok(canonical)
    }
}
