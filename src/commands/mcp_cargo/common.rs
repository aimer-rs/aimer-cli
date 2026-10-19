use std::path::{Path, PathBuf};
use std::time::Duration;

use toml_edit::DocumentMut;

pub(super) const CARGO_TIMEOUT: Duration = Duration::from_secs(300);
pub(super) const MAX_TOOL_OUTPUT_BYTES: usize = 32 * 1024;

pub(super) fn find_lockfile(root: &Path) -> Option<PathBuf> {
    let mut current = root.canonicalize().ok()?;
    let mut nearest_manifest = None;
    loop {
        let manifest = current.join("Cargo.toml");
        if manifest.is_file() {
            nearest_manifest.get_or_insert_with(|| current.join("Cargo.lock"));
            if std::fs::read_to_string(&manifest)
                .ok()
                .and_then(|source| source.parse::<DocumentMut>().ok())
                .is_some_and(|document| document.as_table().contains_key("workspace"))
            {
                return Some(current.join("Cargo.lock"));
            }
        }
        if !current.pop() {
            return nearest_manifest;
        }
    }
}
pub(super) fn bounded_text(bytes: &[u8]) -> (String, bool) {
    let truncated = bytes.len() > MAX_TOOL_OUTPUT_BYTES;
    let bytes = &bytes[..bytes.len().min(MAX_TOOL_OUTPUT_BYTES)];
    (String::from_utf8_lossy(bytes).into_owned(), truncated)
}
