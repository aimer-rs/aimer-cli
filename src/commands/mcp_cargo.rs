//! Cargo project and manifest operations exposed by the MCP server.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tempfile::TempDir;
use tokio::process::Command;
use tokio::time::{interval, timeout};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value as TomlValue};

use super::mcp_ra::McpProgressReporter;

const CARGO_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_TOOL_OUTPUT_BYTES: usize = 32 * 1024;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_MANIFEST_OPERATIONS: usize = 50;
const MAX_METADATA_ITEMS: usize = 200;
const MAX_STAGED_FILES: usize = 20_000;
const MAX_STAGED_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
pub struct CargoRunArguments {
    /// Optional Cargo package name. Omit to run across the selected workspace.
    #[serde(default)]
    pub package: Option<String>,
    /// Optional target triple, for example `x86_64-unknown-linux-gnu`.
    #[serde(default)]
    pub target: Option<String>,
    /// Features to enable.
    #[serde(default)]
    pub features: Vec<String>,
    /// Enable every available feature.
    #[serde(default)]
    pub all_features: bool,
    /// Disable default features.
    #[serde(default)]
    pub no_default_features: bool,
    /// Require Cargo.lock to remain current. Defaults to Cargo's normal behavior.
    #[serde(default)]
    pub locked: bool,
    /// Optional Cargo test name filter. Used by `aimer_rust_cargo_test` only.
    #[serde(default)]
    pub test_filter: Option<String>,
}

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

pub async fn read_project_metadata(root: &Path, allow_network: bool) -> anyhow::Result<Value> {
    let manifest = resolve_manifest(root, None)?;
    let mut command = Command::new("cargo");
    command
        .args(["metadata", "--format-version", "1", "--locked"])
        .arg("--manifest-path")
        .arg(&manifest)
        .current_dir(root)
        .env(
            "CARGO_NET_OFFLINE",
            if allow_network { "false" } else { "true" },
        )
        .kill_on_drop(true);
    let output = match timeout(CARGO_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return Ok(json!({
                "status": "unavailable",
                "message": format!("could not start cargo metadata: {error}")
            }));
        }
        Err(_) => {
            return Ok(json!({
                "status": "unavailable",
                "message": "cargo metadata exceeded the 300 second timeout"
            }));
        }
    };
    if !output.status.success() {
        return Ok(json!({
            "status": "unavailable",
            "exit_code": output.status.code(),
            "message": bounded_text(&output.stderr).0
        }));
    }
    let metadata: Value =
        serde_json::from_slice(&output.stdout).context("decoding cargo metadata JSON")?;
    Ok(project_metadata_value(root, &metadata))
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

pub async fn run_cargo(
    root: &Path,
    allow_network: bool,
    is_test: bool,
    arguments: CargoRunArguments,
    progress: Option<&McpProgressReporter>,
) -> anyhow::Result<Value> {
    validate_cargo_arguments(is_test, &arguments)?;
    let command_name = if is_test { "test" } else { "check" };
    let lockfile = find_lockfile(root);
    let lock_before = match &lockfile {
        Some(path) => tokio::fs::read(path).await.ok(),
        None => None,
    };

    let mut command = Command::new("cargo");
    command
        .arg(command_name)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env(
            "CARGO_NET_OFFLINE",
            if allow_network { "false" } else { "true" },
        )
        .kill_on_drop(true);
    if let Some(package) = arguments.package.as_deref() {
        command.args(["--package", package]);
    } else {
        command.arg("--workspace");
    }
    if let Some(target) = arguments.target.as_deref() {
        command.args(["--target", target]);
    }
    if !arguments.features.is_empty() {
        command.args(["--features", &arguments.features.join(",")]);
    }
    if arguments.all_features {
        command.arg("--all-features");
    }
    if arguments.no_default_features {
        command.arg("--no-default-features");
    }
    if arguments.locked {
        command.arg("--locked");
    }
    if let Some(filter) = arguments.test_filter.as_deref() {
        command.arg(filter);
    }

    let command_label = format!("cargo {command_name}");
    if let Some(progress) = progress {
        progress
            .report(format!(
                "Running {command_label} for the selected Rust workspace."
            ))
            .await;
    }
    let started = Instant::now();
    let output_future = timeout(CARGO_TIMEOUT, command.output());
    tokio::pin!(output_future);
    let mut progress_interval = interval(Duration::from_secs(10));
    progress_interval.tick().await;
    let output_result = loop {
        tokio::select! {
            result = &mut output_future => break result,
            _ = progress_interval.tick() => {
                if let Some(progress) = progress {
                    progress.report(format!("{command_label} is still running.")).await;
                }
            }
        }
    };
    let (status, exit_code, stdout, stderr) = match output_result {
        Ok(Ok(output)) => (
            if output.status.success() {
                "passed"
            } else {
                "failed"
            },
            output.status.code(),
            bounded_text(&output.stdout),
            bounded_text(&output.stderr),
        ),
        Ok(Err(error)) => return Err(error).context(format!("starting {command_label}")),
        Err(_) => (
            "timed_out",
            None,
            (String::new(), false),
            (String::new(), false),
        ),
    };
    let lockfile_changed = if let Some(path) = lockfile {
        let lock_after = tokio::fs::read(path).await.ok();
        lock_before != lock_after
    } else {
        false
    };
    Ok(json!({
        "operation": command_name,
        "status": status,
        "exit_code": exit_code,
        "duration_ms": started.elapsed().as_millis(),
        "stdout": stdout.0,
        "stdout_truncated": stdout.1,
        "stderr": stderr.0,
        "stderr_truncated": stderr.1,
        "lockfile_changed": lockfile_changed,
        "timeout_seconds": CARGO_TIMEOUT.as_secs()
    }))
}

fn validate_cargo_arguments(is_test: bool, arguments: &CargoRunArguments) -> anyhow::Result<()> {
    anyhow::ensure!(
        arguments
            .package
            .as_deref()
            .is_none_or(|value| !value.trim().is_empty()),
        "package must not be empty"
    );
    anyhow::ensure!(
        arguments
            .package
            .as_deref()
            .is_none_or(|value| !value.starts_with('-')),
        "package must be a package name, not an option"
    );
    anyhow::ensure!(
        arguments
            .target
            .as_deref()
            .is_none_or(|value| !value.trim().is_empty()),
        "target must not be empty"
    );
    anyhow::ensure!(
        arguments
            .target
            .as_deref()
            .is_none_or(|value| !value.starts_with('-')),
        "target must be a target triple, not an option"
    );
    anyhow::ensure!(
        arguments
            .features
            .iter()
            .all(|feature| !feature.trim().is_empty() && !feature.starts_with('-')),
        "features must be non-empty feature names, not options"
    );
    anyhow::ensure!(
        arguments
            .test_filter
            .as_deref()
            .is_none_or(|value| !value.trim().is_empty()),
        "test_filter must not be empty"
    );
    anyhow::ensure!(
        arguments
            .test_filter
            .as_deref()
            .is_none_or(|value| !value.starts_with('-')),
        "test_filter must be a test name, not an option"
    );
    anyhow::ensure!(
        is_test || arguments.test_filter.is_none(),
        "test_filter is only valid for aimer_rust_cargo_test"
    );
    anyhow::ensure!(
        !(arguments.all_features && !arguments.features.is_empty()),
        "features and all_features cannot be used together"
    );
    Ok(())
}

fn project_metadata_value(root: &Path, metadata: &Value) -> Value {
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let packages_truncated = packages.len() > MAX_METADATA_ITEMS;
    let packages = packages
        .iter()
        .take(MAX_METADATA_ITEMS)
        .map(|package| {
            let mut result = Map::new();
            for key in [
                "id",
                "name",
                "version",
                "source",
                "edition",
                "features",
                "dependencies",
                "targets",
            ] {
                if let Some(value) = package.get(key) {
                    result.insert(key.to_owned(), value.clone());
                }
            }
            if let Some(path) = package.get("manifest_path").and_then(Value::as_str) {
                result.insert(
                    "manifest_path".to_owned(),
                    json!(normalize_path(root, path)),
                );
            }
            Value::Object(result)
        })
        .collect::<Vec<_>>();
    let resolve = metadata.get("resolve").cloned().unwrap_or(Value::Null);
    let nodes = resolve
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let nodes_truncated = nodes.len() > MAX_METADATA_ITEMS;
    let resolve = if let Some(resolve) = resolve.as_object() {
        json!({
            "root": resolve.get("root").cloned().unwrap_or(Value::Null),
            "nodes": nodes.into_iter().take(MAX_METADATA_ITEMS).collect::<Vec<_>>(),
            "nodes_truncated": nodes_truncated
        })
    } else {
        Value::Null
    };
    json!({
        "status": "ready",
        "workspace_root": metadata.get("workspace_root").and_then(Value::as_str).map(|path| normalize_path(root, path)),
        "workspace_members": bounded_array(metadata.get("workspace_members")),
        "workspace_default_members": bounded_array(metadata.get("workspace_default_members")),
        "packages": packages,
        "packages_truncated": packages_truncated,
        "resolve": resolve
    })
}

fn bounded_array(value: Option<&Value>) -> Value {
    let Some(items) = value.and_then(Value::as_array) else {
        return json!({ "results": [], "truncated": false });
    };
    json!({
        "results": items.iter().take(MAX_METADATA_ITEMS).cloned().collect::<Vec<_>>(),
        "truncated": items.len() > MAX_METADATA_ITEMS
    })
}

fn normalize_path(root: &Path, path: &str) -> String {
    let path = Path::new(path);
    path.strip_prefix(root)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

fn resolve_manifest(root: &Path, requested: Option<&str>) -> anyhow::Result<PathBuf> {
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

fn relative_path(root: &Path, path: &Path) -> anyhow::Result<String> {
    Ok(path
        .strip_prefix(root.canonicalize().context("resolving project root")?)?
        .to_string_lossy()
        .replace('\\', "/"))
}

fn apply_manifest_operation(
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

fn json_to_toml(value: &Value) -> anyhow::Result<TomlValue> {
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

fn set_item(table: &mut Table, path: &[String], item: Item) -> anyhow::Result<()> {
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

fn set_inline_item(
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

fn remove_item(table: &mut Table, path: &[String]) -> anyhow::Result<()> {
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

fn remove_inline_item(table: &mut toml_edit::InlineTable, path: &[String]) -> anyhow::Result<()> {
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

fn is_manifest_error(stderr: &str) -> bool {
    let message = stderr.to_ascii_lowercase();
    message.contains("failed to parse manifest")
        || message.contains("invalid type")
        || message.contains("unknown variant")
        || message.contains("unused manifest key")
        || message.contains("failed to load manifest for workspace member")
}

fn stage_workspace(root: &Path, manifest: &Path, candidate: &[u8]) -> anyhow::Result<TempDir> {
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

fn copy_workspace_tree(
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

fn find_lockfile(root: &Path) -> Option<PathBuf> {
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

fn bounded_text(bytes: &[u8]) -> (String, bool) {
    let truncated = bytes.len() > MAX_TOOL_OUTPUT_BYTES;
    let bytes = &bytes[..bytes.len().min(MAX_TOOL_OUTPUT_BYTES)];
    (String::from_utf8_lossy(bytes).into_owned(), truncated)
}

#[cfg(test)]
mod tests {
    use super::{
        ManifestOperation, ManifestOperationAction, apply_manifest_operation,
        project_metadata_value, resolve_manifest,
    };
    use serde_json::json;
    use std::fs;
    use toml_edit::DocumentMut;

    #[test]
    fn generic_manifest_edits_preserve_comments_and_replace_values() {
        let mut document = "# keep this comment\n[dependencies]\nserde = \"1\"\n"
            .parse::<DocumentMut>()
            .unwrap();
        apply_manifest_operation(
            &mut document,
            &ManifestOperation {
                action: ManifestOperationAction::Set,
                path: vec!["dependencies".to_owned(), "serde".to_owned()],
                value: Some(json!({ "version": "2", "features": ["derive"] })),
            },
        )
        .unwrap();
        let source = document.to_string();
        assert!(source.contains("# keep this comment"));
        assert!(source.contains("serde"));
        let parsed = toml_edit::de::from_str::<serde_json::Value>(&source).unwrap();
        assert_eq!(parsed["dependencies"]["serde"]["version"], "2");
        assert_eq!(
            parsed["dependencies"]["serde"]["features"],
            json!(["derive"])
        );
    }

    #[test]
    fn nested_edit_updates_inline_dependency_tables() {
        let mut document = "[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\n"
            .parse::<DocumentMut>()
            .unwrap();
        apply_manifest_operation(
            &mut document,
            &ManifestOperation {
                action: ManifestOperationAction::Set,
                path: vec![
                    "dependencies".to_owned(),
                    "serde".to_owned(),
                    "features".to_owned(),
                ],
                value: Some(json!(["derive", "rc"])),
            },
        )
        .unwrap();
        let parsed = toml_edit::de::from_str::<serde_json::Value>(&document.to_string()).unwrap();
        assert_eq!(
            parsed["dependencies"]["serde"]["features"],
            json!(["derive", "rc"])
        );
    }

    #[test]
    fn manifest_remove_requires_existing_path() {
        let mut document = "[features]\ndefault = []\n".parse::<DocumentMut>().unwrap();
        let error = apply_manifest_operation(
            &mut document,
            &ManifestOperation {
                action: ManifestOperationAction::Remove,
                path: vec!["features".to_owned(), "missing".to_owned()],
                value: None,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("does not exist"));
    }

    #[test]
    fn manifest_path_must_remain_inside_selected_project() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(
            temporary.path().join("Cargo.toml"),
            "[package]\nname='x'\nversion='0.1.0'\n",
        )
        .unwrap();
        assert!(resolve_manifest(temporary.path(), Some("../Cargo.toml")).is_err());
    }

    #[test]
    fn metadata_projection_bounds_package_and_dependency_graph_lists() {
        let packages = (0..205)
            .map(|index| {
                json!({
                    "id": format!("pkg-{index}"),
                    "name": format!("pkg-{index}"),
                    "manifest_path": "/workspace/Cargo.toml",
                    "targets": [], "features": {}, "dependencies": []
                })
            })
            .collect::<Vec<_>>();
        let nodes = (0..205)
            .map(|index| json!({ "id": format!("pkg-{index}") }))
            .collect::<Vec<_>>();
        let result = project_metadata_value(
            std::path::Path::new("/workspace"),
            &json!({
                "workspace_root": "/workspace",
                "workspace_members": [],
                "workspace_default_members": [],
                "packages": packages,
                "resolve": { "root": null, "nodes": nodes }
            }),
        );
        assert_eq!(result["packages"].as_array().unwrap().len(), 200);
        assert_eq!(result["packages_truncated"], true);
        assert_eq!(result["resolve"]["nodes"].as_array().unwrap().len(), 200);
        assert_eq!(result["resolve"]["nodes_truncated"], true);
    }

    #[test]
    fn cargo_output_is_bounded_and_reports_truncation() {
        let output = vec![b'x'; super::MAX_TOOL_OUTPUT_BYTES + 10];
        let (text, truncated) = super::bounded_text(&output);
        assert_eq!(text.len(), super::MAX_TOOL_OUTPUT_BYTES);
        assert!(truncated);
    }

    #[test]
    fn cargo_arguments_reject_ambiguous_or_option_like_values() {
        use super::{CargoRunArguments, validate_cargo_arguments};

        let error = validate_cargo_arguments(
            false,
            &CargoRunArguments {
                test_filter: Some("smoke".to_owned()),
                ..CargoRunArguments::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("only valid"));

        let error = validate_cargo_arguments(
            true,
            &CargoRunArguments {
                features: vec!["serde".to_owned()],
                all_features: true,
                ..CargoRunArguments::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("cannot be used together"));
    }

    #[tokio::test]
    async fn cargo_tools_return_real_check_and_test_status_and_lockfile_change() {
        use super::{CargoRunArguments, run_cargo};

        let temporary = tempfile::tempdir().unwrap();
        fs::create_dir(temporary.path().join("src")).unwrap();
        fs::write(
            temporary.path().join("Cargo.toml"),
            "[package]\nname = \"mcp-cargo-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            temporary.path().join("src/lib.rs"),
            "#[test]\nfn smoke() {}\npub fn value() -> u32 { 42 }\n",
        )
        .unwrap();

        let checked = run_cargo(
            temporary.path(),
            false,
            false,
            CargoRunArguments::default(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(checked["status"], "passed");
        assert_eq!(checked["exit_code"], 0);
        assert_eq!(checked["lockfile_changed"], true);

        fs::write(
            temporary.path().join("src/lib.rs"),
            "pub fn value() -> u32 { \"not a number\" }\n",
        )
        .unwrap();
        let failed = run_cargo(
            temporary.path(),
            false,
            false,
            CargoRunArguments::default(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["exit_code"], 101);

        fs::write(
            temporary.path().join("src/lib.rs"),
            "#[test]\nfn smoke() {}\npub fn value() -> u32 { 42 }\n",
        )
        .unwrap();
        let tested = run_cargo(
            temporary.path(),
            false,
            true,
            CargoRunArguments {
                test_filter: Some("smoke".to_owned()),
                ..CargoRunArguments::default()
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(tested["status"], "passed");
        assert_eq!(tested["exit_code"], 0);
        assert!(tested["stdout"].as_str().unwrap().contains("smoke"));
    }

    #[tokio::test]
    async fn manifest_preview_validates_in_a_staged_workspace_without_touching_project_files() {
        use super::{
            ManifestEditArguments, ManifestOperation, ManifestOperationAction,
            prepare_manifest_edit,
        };

        let temporary = tempfile::tempdir().unwrap();
        fs::create_dir(temporary.path().join("src")).unwrap();
        let manifest = temporary.path().join("Cargo.toml");
        let original =
            "[package]\nname = \"mcp-manifest-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
        fs::write(&manifest, original).unwrap();
        fs::write(temporary.path().join("src/lib.rs"), "pub fn value() {}\n").unwrap();

        let candidate = prepare_manifest_edit(
            temporary.path(),
            false,
            ManifestEditArguments {
                manifest_path: "Cargo.toml".to_owned(),
                operations: vec![ManifestOperation {
                    action: ManifestOperationAction::Set,
                    path: vec!["package".to_owned(), "version".to_owned()],
                    value: Some(json!("0.2.0")),
                }],
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(candidate.validation["status"], "passed");
        assert!(candidate.applicable);
        assert_ne!(candidate.before, candidate.after);
        assert_eq!(fs::read_to_string(&manifest).unwrap(), original);
        assert!(!temporary.path().join("Cargo.lock").exists());
    }

    #[tokio::test]
    async fn metadata_read_does_not_create_a_missing_lockfile() {
        use super::read_project_metadata;

        let temporary = tempfile::tempdir().unwrap();
        fs::create_dir(temporary.path().join("src")).unwrap();
        fs::write(
            temporary.path().join("Cargo.toml"),
            "[package]\nname = \"mcp-metadata-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let metadata = read_project_metadata(temporary.path(), false)
            .await
            .unwrap();
        assert_eq!(metadata["status"], "unavailable");
        assert!(!temporary.path().join("Cargo.lock").exists());
    }

    #[tokio::test]
    async fn manifest_reader_returns_structured_toml() {
        use super::{ManifestReadArguments, read_manifest};

        let temporary = tempfile::tempdir().unwrap();
        fs::write(
            temporary.path().join("Cargo.toml"),
            "[package]\nname = \"mcp-manifest-reader\"\nversion = \"0.1.0\"\n[features]\ndefault = []\n",
        )
        .unwrap();
        let result = read_manifest(
            temporary.path(),
            ManifestReadArguments {
                manifest_path: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(result["manifest_path"], "Cargo.toml");
        assert_eq!(result["manifest"]["package"]["name"], "mcp-manifest-reader");
        assert_eq!(result["manifest"]["features"]["default"], json!([]));
    }

    #[tokio::test]
    async fn cargo_manifest_validation_error_makes_preview_non_applicable() {
        use super::{
            ManifestEditArguments, ManifestOperation, ManifestOperationAction,
            prepare_manifest_edit,
        };

        let temporary = tempfile::tempdir().unwrap();
        fs::create_dir(temporary.path().join("src")).unwrap();
        fs::write(
            temporary.path().join("Cargo.toml"),
            "[package]\nname = \"mcp-invalid-manifest\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(temporary.path().join("src/lib.rs"), "pub fn value() {}\n").unwrap();
        let candidate = prepare_manifest_edit(
            temporary.path(),
            false,
            ManifestEditArguments {
                manifest_path: "Cargo.toml".to_owned(),
                operations: vec![ManifestOperation {
                    action: ManifestOperationAction::Set,
                    path: vec!["package".to_owned(), "version".to_owned()],
                    value: Some(json!(7)),
                }],
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(candidate.validation["status"], "failed");
        assert!(!candidate.applicable);
        assert!(!candidate.validation["message"].as_str().unwrap().is_empty());
        assert!(!temporary.path().join("Cargo.lock").exists());
    }

    #[tokio::test]
    async fn project_metadata_reads_locked_workspace_without_changing_lockfile() {
        use super::read_project_metadata;

        let temporary = tempfile::tempdir().unwrap();
        fs::create_dir(temporary.path().join("src")).unwrap();
        fs::write(
            temporary.path().join("Cargo.toml"),
            "[package]\nname = \"mcp-metadata-ready\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(temporary.path().join("src/lib.rs"), "pub fn value() {}\n").unwrap();
        fs::write(
            temporary.path().join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"mcp-metadata-ready\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let original_lock = fs::read(temporary.path().join("Cargo.lock")).unwrap();
        let metadata = read_project_metadata(temporary.path(), false)
            .await
            .unwrap();
        assert_eq!(metadata["status"], "ready");
        assert_eq!(metadata["packages"][0]["name"], "mcp-metadata-ready");
        assert_eq!(
            fs::read(temporary.path().join("Cargo.lock")).unwrap(),
            original_lock
        );
    }
}
