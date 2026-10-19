use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Context;
use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::process::Command;
use tokio::time::timeout;

use super::common::{CARGO_TIMEOUT, bounded_text};
use super::manifest::resolve_manifest;

const MAX_METADATA_ITEMS: usize = 200;

#[derive(Debug, Clone, Copy, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectMetadataDetail {
    #[default]
    Summary,
    Full,
}

#[cfg(test)]
pub async fn read_project_metadata(root: &Path, allow_network: bool) -> anyhow::Result<Value> {
    read_project_metadata_with_detail(root, allow_network, ProjectMetadataDetail::Full).await
}

pub async fn read_project_metadata_with_detail(
    root: &Path,
    allow_network: bool,
    detail: ProjectMetadataDetail,
) -> anyhow::Result<Value> {
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
    Ok(match detail {
        ProjectMetadataDetail::Summary => project_metadata_summary_value(root, &metadata),
        ProjectMetadataDetail::Full => project_metadata_value(root, &metadata),
    })
}

pub(super) fn project_metadata_value(root: &Path, metadata: &Value) -> Value {
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

pub(super) fn project_metadata_summary_value(root: &Path, metadata: &Value) -> Value {
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let member_ids = metadata
        .get("workspace_members")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let default_member_ids = metadata
        .get("workspace_default_members")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();

    let mut members = Vec::new();
    let mut default_members = Vec::new();
    for package in packages.iter().filter(|package| {
        package
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| member_ids.contains(id))
    }) {
        let Some(name) = package.get("name").and_then(Value::as_str) else {
            continue;
        };
        if package
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| default_member_ids.contains(id))
        {
            default_members.push(name.to_owned());
        }
        if members.len() == MAX_METADATA_ITEMS {
            continue;
        }

        let targets = package
            .get("targets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|target| {
                Some(json!({
                    "name": target.get("name")?.as_str()?,
                    "kind": target.get("kind").cloned().unwrap_or_else(|| json!([]))
                }))
            })
            .collect::<Vec<_>>();
        let dependencies = package
            .get("dependencies")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|dependency| {
                dependency
                    .get("rename")
                    .and_then(Value::as_str)
                    .or_else(|| dependency.get("name").and_then(Value::as_str))
                    .map(str::to_owned)
            })
            .collect::<BTreeSet<_>>();
        let features = package
            .get("features")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|features| features.keys().cloned())
            .collect::<Vec<_>>();

        let mut summary = json!({
            "name": name,
            "version": package.get("version").and_then(Value::as_str),
            "manifest_path": package
                .get("manifest_path")
                .and_then(Value::as_str)
                .map(|path| normalize_path(root, path)),
            "targets": targets,
            "dependencies": dependencies.into_iter().collect::<Vec<_>>()
        });
        if let Some(edition) = package.get("edition").and_then(Value::as_str) {
            summary["edition"] = json!(edition);
        }
        if !features.is_empty() {
            summary["features"] = json!(features);
        }
        members.push(summary);
    }
    members.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    default_members.sort();

    let workspace_root = metadata
        .get("workspace_root")
        .and_then(Value::as_str)
        .map(|path| normalize_path(root, path));
    json!({
        "status": "ready",
        "detail": "summary",
        "workspace_root": workspace_root,
        "workspace_package_count": member_ids.len(),
        "dependency_package_count": packages.len().saturating_sub(member_ids.len()),
        "default_members": default_members,
        "packages": members,
        "packages_truncated": member_ids.len() > MAX_METADATA_ITEMS
    })
}

pub(super) fn bounded_array(value: Option<&Value>) -> Value {
    let Some(items) = value.and_then(Value::as_array) else {
        return json!({ "results": [], "truncated": false });
    };
    json!({
        "results": items.iter().take(MAX_METADATA_ITEMS).cloned().collect::<Vec<_>>(),
        "truncated": items.len() > MAX_METADATA_ITEMS
    })
}

pub(super) fn normalize_path(root: &Path, path: &str) -> String {
    let path = Path::new(path);
    let normalized = path
        .strip_prefix(root)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().into_owned());
    if normalized.is_empty() {
        ".".to_owned()
    } else {
        normalized
    }
}
