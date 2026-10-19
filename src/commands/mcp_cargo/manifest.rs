use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::process::Command;
use tokio::time::{interval, timeout};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value as TomlValue};

use super::super::mcp_ra::McpProgressReporter;
use super::common::{CARGO_TIMEOUT, bounded_text};

const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_MANIFEST_OPERATIONS: usize = 50;
const MAX_STAGED_FILES: usize = 20_000;
const MAX_STAGED_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ManifestReadArguments {
    /// Optional manifest path relative to the selected Rust project. Defaults to Cargo.toml at its root.
    #[serde(default)]
    pub manifest_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ManifestEditArguments {
    /// Existing Cargo.toml path relative to the selected Rust project.
    pub manifest_path: String,
    /// Ordered key-path edits. Set replaces or creates a TOML value; remove deletes an existing value.
    pub operations: Vec<ManifestOperation>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ManifestOperation {
    /// Whether to set or remove the selected TOML value.
    pub action: ManifestOperationAction,
    /// TOML table/key path, for example `["dependencies", "serde"]`.
    pub path: Vec<String>,
    /// TOML-compatible JSON value, required for `set` and omitted for `remove`.
    #[serde(default)]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManifestOperationAction {
    Set,
    Remove,
}

pub async fn read_manifest(root: &Path, arguments: ManifestReadArguments) -> anyhow::Result<Value> {
    let manifest = resolve_manifest(root, arguments.manifest_path.as_deref())?;
    let contents = tokio::fs::read_to_string(&manifest)
        .await
        .with_context(|| format!("reading manifest '{}'", manifest.display()))?;
    if contents.len() > MAX_MANIFEST_BYTES {
        bail!("manifest exceeds the {MAX_MANIFEST_BYTES}-byte limit");
    }
    let parsed = toml_edit::de::from_str::<Value>(&contents)
        .with_context(|| format!("parsing manifest '{}'", manifest.display()))?;
    Ok(json!({
        "manifest_path": relative_path(root, &manifest)?,
        "manifest": parsed
    }))
}

pub async fn prepare_manifest_edit(
    root: &Path,
    allow_network: bool,
    arguments: ManifestEditArguments,
    progress: Option<&McpProgressReporter>,
) -> anyhow::Result<ManifestCandidate> {
    anyhow::ensure!(
        !arguments.operations.is_empty() && arguments.operations.len() <= MAX_MANIFEST_OPERATIONS,
        "operations must contain 1–{MAX_MANIFEST_OPERATIONS} items"
    );
    let manifest = resolve_manifest(root, Some(&arguments.manifest_path))?;
    let before = tokio::fs::read(&manifest)
        .await
        .with_context(|| format!("reading manifest '{}'", manifest.display()))?;
    anyhow::ensure!(
        before.len() <= MAX_MANIFEST_BYTES,
        "manifest exceeds the {MAX_MANIFEST_BYTES}-byte limit"
    );
    let source = std::str::from_utf8(&before).context("manifest is not valid UTF-8")?;
    let mut document = source
        .parse::<DocumentMut>()
        .with_context(|| format!("parsing manifest '{}'", manifest.display()))?;
    for (index, operation) in arguments.operations.iter().enumerate() {
        apply_manifest_operation(&mut document, operation)
            .with_context(|| format!("applying manifest operation {}", index + 1))?;
    }
    let after = document.to_string().into_bytes();
    anyhow::ensure!(
        after.len() <= MAX_MANIFEST_BYTES,
        "edited manifest exceeds the {MAX_MANIFEST_BYTES}-byte limit"
    );

    if let Some(progress) = progress {
        progress
            .report("Staging the Cargo workspace and validating the proposed manifest.")
            .await;
    }
    let (validation, applicable) =
        validate_staged_manifest(root, &manifest, &after, allow_network, progress).await;
    Ok(ManifestCandidate {
        manifest,
        before,
        after,
        validation,
        applicable,
    })
}

#[derive(Debug)]
pub struct ManifestCandidate {
    pub manifest: PathBuf,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
    pub validation: Value,
    pub applicable: bool,
}

pub(super) fn resolve_manifest(root: &Path, requested: Option<&str>) -> anyhow::Result<PathBuf> {
    let root = root
        .canonicalize()
        .context("resolving selected Rust project")?;
    let path = match requested {
        Some(path) if !path.trim().is_empty() => {
            anyhow::ensure!(
                !Path::new(path).is_absolute(),
                "manifest_path must be relative to the selected Rust project"
            );
            root.join(path)
        }
        Some(_) => bail!("manifest_path must not be empty"),
        None => root.join("Cargo.toml"),
    };
    let resolved = path
        .canonicalize()
        .with_context(|| format!("resolving manifest '{}'", path.display()))?;
    anyhow::ensure!(
        resolved.starts_with(&root),
        "manifest_path must stay inside the selected Rust project"
    );
    anyhow::ensure!(
        resolved
            .file_name()
            .is_some_and(|name| name == "Cargo.toml"),
        "manifest_path must name Cargo.toml"
    );
    anyhow::ensure!(resolved.is_file(), "manifest_path must be a file");
    Ok(resolved)
}

pub(super) fn relative_path(root: &Path, path: &Path) -> anyhow::Result<String> {
    Ok(path
        .strip_prefix(root.canonicalize().context("resolving project root")?)?
        .to_string_lossy()
        .replace('\\', "/"))
}

pub(super) fn apply_manifest_operation(
    document: &mut DocumentMut,
    operation: &ManifestOperation,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !operation.path.is_empty(),
        "path must contain at least one TOML key"
    );
    anyhow::ensure!(
        operation.path.len() <= 16,
        "path must contain at most 16 TOML keys"
    );
    anyhow::ensure!(
        operation.path.iter().all(|part| !part.trim().is_empty()),
        "path keys must not be empty"
    );
    match operation.action {
        ManifestOperationAction::Set => {
            let value = operation
                .value
                .as_ref()
                .ok_or_else(|| anyhow!("set operation requires value"))?;
            let item = Item::Value(json_to_toml(value)?);
            set_item(document.as_table_mut(), &operation.path, item)
        }
        ManifestOperationAction::Remove => {
            anyhow::ensure!(
                operation.value.is_none(),
                "remove operation must omit value"
            );
            remove_item(document.as_table_mut(), &operation.path)
        }
    }
}

pub(super) fn json_to_toml(value: &Value) -> anyhow::Result<TomlValue> {
    match value {
        Value::Null => bail!("null is not a TOML value"),
        Value::Bool(value) => Ok(TomlValue::from(*value)),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(TomlValue::from(value))
            } else if let Some(value) = value.as_u64() {
                let value = i64::try_from(value)
                    .context("TOML integers must fit in a signed 64-bit value")?;
                Ok(TomlValue::from(value))
            } else if let Some(value) = value.as_f64() {
                Ok(TomlValue::from(value))
            } else {
                bail!("number cannot be represented as TOML")
            }
        }
        Value::String(value) => Ok(TomlValue::from(value.clone())),
        Value::Array(values) => {
            let mut array = Array::new();
            for value in values {
                array.push(json_to_toml(value)?);
            }
            Ok(TomlValue::Array(array))
        }
        Value::Object(values) => {
            let mut table = InlineTable::new();
            for (key, value) in values {
                table.insert(key, json_to_toml(value)?);
            }
            Ok(TomlValue::InlineTable(table))
        }
    }
}

pub(super) fn set_item(table: &mut Table, path: &[String], item: Item) -> anyhow::Result<()> {
    let key = &path[0];
    if path.len() == 1 {
        table.insert(key, item);
        return Ok(());
    }
    if !table.contains_key(key) {
        table.insert(key, Item::Table(Table::new()));
    }
    match table.get_mut(key).expect("just inserted or checked") {
        Item::Table(table) => set_item(table, &path[1..], item),
        Item::Value(TomlValue::InlineTable(table)) => set_inline_item(table, &path[1..], item),
        _ => bail!("TOML key '{}' is not a table", key),
    }
}

pub(super) fn set_inline_item(
    table: &mut toml_edit::InlineTable,
    path: &[String],
    item: Item,
) -> anyhow::Result<()> {
    let key = &path[0];
    if path.len() == 1 {
        let Item::Value(value) = item else {
            bail!("nested tables cannot be inserted into an inline table");
        };
        table.insert(key, value);
        return Ok(());
    }
    if !table.contains_key(key) {
        table.insert(key, TomlValue::InlineTable(toml_edit::InlineTable::new()));
    }
    match table.get_mut(key).expect("just inserted or checked") {
        TomlValue::InlineTable(table) => set_inline_item(table, &path[1..], item),
        _ => bail!("TOML key '{}' is not a table", key),
    }
}

pub(super) fn remove_item(table: &mut Table, path: &[String]) -> anyhow::Result<()> {
    let key = &path[0];
    if path.len() == 1 {
        anyhow::ensure!(
            table.remove(key).is_some(),
            "TOML key '{}' does not exist",
            key
        );
        return Ok(());
    }
    let child = table
        .get_mut(key)
        .ok_or_else(|| anyhow!("TOML key '{}' does not exist", key))?;
    match child {
        Item::Table(table) => remove_item(table, &path[1..]),
        Item::Value(TomlValue::InlineTable(table)) => remove_inline_item(table, &path[1..]),
        _ => bail!("TOML key '{}' is not a table", key),
    }
}

pub(super) fn remove_inline_item(
    table: &mut toml_edit::InlineTable,
    path: &[String],
) -> anyhow::Result<()> {
    let key = &path[0];
    if path.len() == 1 {
        anyhow::ensure!(
            table.remove(key).is_some(),
            "TOML key '{}' does not exist",
            key
        );
        return Ok(());
    }
    match table.get_mut(key) {
        Some(TomlValue::InlineTable(table)) => remove_inline_item(table, &path[1..]),
        Some(_) => bail!("TOML key '{}' is not a table", key),
        None => bail!("TOML key '{}' does not exist", key),
    }
}

async fn validate_staged_manifest(
    root: &Path,
    manifest: &Path,
    candidate: &[u8],
    allow_network: bool,
    progress: Option<&McpProgressReporter>,
) -> (Value, bool) {
    let stage = match stage_workspace(root, manifest, candidate) {
        Ok(stage) => stage,
        Err(error) => {
            return (
                json!({ "status": "unavailable", "message": format!("could not stage workspace: {error:#}") }),
                true,
            );
        }
    };
    let staged_manifest = stage.path().join("workspace").join(
        manifest
            .strip_prefix(root)
            .unwrap_or_else(|_| Path::new("Cargo.toml")),
    );
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .arg("--manifest-path")
        .arg(&staged_manifest)
        .current_dir(stage.path().join("workspace"))
        .env(
            "CARGO_NET_OFFLINE",
            if allow_network { "false" } else { "true" },
        )
        .kill_on_drop(true)
        .output();
    if let Some(progress) = progress {
        progress
            .report("Running cargo metadata against the temporary manifest candidate.")
            .await;
    }
    let output_future = timeout(CARGO_TIMEOUT, output);
    tokio::pin!(output_future);
    let mut progress_interval = interval(Duration::from_secs(10));
    progress_interval.tick().await;
    let output_result = loop {
        tokio::select! {
            result = &mut output_future => break result,
            _ = progress_interval.tick(), if progress.is_some() => {
                if let Some(progress) = progress {
                    progress.report("Cargo metadata validation is still running.").await;
                }
            }
        }
    };
    match output_result {
        Ok(Ok(output)) if output.status.success() => (json!({ "status": "passed" }), true),
        Ok(Ok(output)) => {
            let message = bounded_text(&output.stderr).0;
            if is_manifest_error(&message) {
                (json!({ "status": "failed", "message": message }), false)
            } else {
                (json!({ "status": "unavailable", "message": message }), true)
            }
        }
        Ok(Err(error)) => (
            json!({ "status": "unavailable", "message": format!("could not start cargo metadata: {error}") }),
            true,
        ),
        Err(_) => (
            json!({ "status": "unavailable", "message": "cargo metadata exceeded the 300 second timeout" }),
            true,
        ),
    }
}

pub(super) fn is_manifest_error(stderr: &str) -> bool {
    let message = stderr.to_ascii_lowercase();
    message.contains("failed to parse manifest")
        || message.contains("invalid type")
        || message.contains("unknown variant")
        || message.contains("unused manifest key")
        || message.contains("failed to load manifest for workspace member")
}

pub(super) fn stage_workspace(
    root: &Path,
    manifest: &Path,
    candidate: &[u8],
) -> anyhow::Result<TempDir> {
    let root = root
        .canonicalize()
        .context("resolving selected Rust project")?;
    let stage = tempfile::Builder::new()
        .prefix("aimer-mcp-validation-")
        .tempdir()
        .context("creating temporary Cargo workspace")?;
    let staged_root = stage.path().join("workspace");
    std::fs::create_dir_all(&staged_root).context("creating staged workspace root")?;
    let mut copied_files = 0;
    let mut copied_bytes = 0;
    copy_workspace_tree(&root, &staged_root, &mut copied_files, &mut copied_bytes)?;
    let relative_manifest = manifest
        .strip_prefix(&root)
        .context("manifest is outside selected project")?;
    let staged_manifest = staged_root.join(relative_manifest);
    std::fs::write(&staged_manifest, candidate).context("writing candidate manifest to staging")?;
    Ok(stage)
}

pub(super) fn copy_workspace_tree(
    source: &Path,
    destination: &Path,
    copied_files: &mut usize,
    copied_bytes: &mut u64,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(source)
        .with_context(|| format!("reading workspace directory '{}'", source.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if matches!(
            name_text.as_ref(),
            ".git"
                | "target"
                | ".aimer"
                | ".codegraph"
                | ".aws"
                | ".idea"
                | "node_modules"
                | "results"
                | "dist"
        ) || (name_text.starts_with('.') && name_text != ".cargo")
        {
            continue;
        }
        let metadata = entry.file_type()?;
        if metadata.is_symlink() {
            continue;
        }
        let target = destination.join(&name);
        if metadata.is_dir() {
            std::fs::create_dir_all(&target)?;
            copy_workspace_tree(&entry.path(), &target, copied_files, copied_bytes)?;
        } else if metadata.is_file()
            && (name_text == "Cargo.toml"
                || name_text == "Cargo.lock"
                || entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "rs")
                || (source
                    .file_name()
                    .is_some_and(|directory| directory == ".cargo")
                    && name_text == "config.toml"))
        {
            let file_size = entry.metadata()?.len();
            anyhow::ensure!(
                *copied_files < MAX_STAGED_FILES,
                "workspace has more than {MAX_STAGED_FILES} Cargo metadata files"
            );
            anyhow::ensure!(
                (*copied_bytes).saturating_add(file_size) <= MAX_STAGED_BYTES,
                "workspace staging exceeds {MAX_STAGED_BYTES} bytes"
            );
            std::fs::copy(entry.path(), target)?;
            *copied_files += 1;
            *copied_bytes += file_size;
        }
    }
    Ok(())
}
