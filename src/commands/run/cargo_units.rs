//! How much work a cargo build has, for the compile percentage.
//!
//! The percentage is `finished / expected`. Both sides have to count the same
//! thing: packages. The `Cargo.lock` package count alone is a poor expectation,
//! because it also lists packages that are never built for the platform being
//! targeted and dev-only dependencies, so the bar stalls well short of 100%. The
//! expectation here is the packages `cargo metadata` resolves for the target,
//! reached through normal and build dependencies only. [`UnitTracker`] counts
//! each package once, however many units cargo builds for it (its build script,
//! a host copy for a proc macro, ...).

use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};
use std::thread;

use serde_json::Value;

/// Counts the packages cargo has finished, each one once.
#[derive(Debug, Default)]
pub struct UnitTracker {
    seen: HashSet<String>,
    anonymous: usize,
}

impl UnitTracker {
    /// Record a finished unit. Returns the number of packages finished so far
    /// when this unit moved the count, and `None` when it did not: a build
    /// script compiles before its package does, and a package already counted
    /// may be built again for another role.
    ///
    /// A unit without a package id is always counted, since there is nothing to
    /// tell it apart from the others.
    pub fn finish(&mut self, package_id: &str, build_script: bool) -> Option<usize> {
        if build_script {
            return None;
        }
        if package_id.is_empty() {
            self.anonymous += 1;
        } else if !self.seen.insert(package_id.to_string()) {
            return None;
        }
        Some(self.seen.len() + self.anonymous)
    }
}

/// Work out the expected package count on a background thread, since reading
/// the dependency graph takes a moment and must not delay the build.
///
/// `platform` is the target triple, or `None` for the host.
pub fn estimate_in_background(platform: Option<String>) -> thread::JoinHandle<usize> {
    thread::spawn(move || expected_packages(platform.as_deref()))
}

/// The number of packages a build for `platform` compiles.
///
/// Falls back to the `Cargo.lock` package count when the dependency graph can't
/// be read, e.g. offline with crates that are not downloaded yet.
pub fn expected_packages(platform: Option<&str>) -> usize {
    let triple = platform.map(str::to_string).or_else(host_triple);
    triple
        .and_then(|triple| resolved_packages(&triple))
        .unwrap_or_else(lock_package_count)
}

fn host_triple() -> Option<String> {
    let output = Command::new("rustc").arg("-vV").output().ok()?;
    String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(|host| host.trim().to_string())
}

fn resolved_packages(triple: &str) -> Option<usize> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--offline"])
        .args(["--filter-platform", triple])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let metadata: Value = serde_json::from_slice(&output.stdout).ok()?;
    reachable_packages(&metadata)
}

/// Count the packages reachable from the root package (or the workspace
/// members when the manifest is virtual) through dependencies that a build
/// uses. Dev-only dependencies are skipped.
fn reachable_packages(metadata: &Value) -> Option<usize> {
    let resolve = metadata.get("resolve")?;
    let nodes: HashMap<&str, &Value> = resolve
        .get("nodes")?
        .as_array()?
        .iter()
        .filter_map(|node| Some((node.get("id")?.as_str()?, node)))
        .collect();

    let mut pending: Vec<&str> = match resolve.get("root").and_then(Value::as_str) {
        Some(root) => vec![root],
        None => metadata
            .get("workspace_members")?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .collect(),
    };

    let mut visited: HashSet<&str> = HashSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let Some(deps) = nodes
            .get(id)
            .and_then(|node| node.get("deps"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for dep in deps {
            if is_build_dependency(dep)
                && let Some(pkg) = dep.get("pkg").and_then(Value::as_str)
            {
                pending.push(pkg);
            }
        }
    }
    (!visited.is_empty()).then_some(visited.len())
}

/// Whether a resolved dependency edge is used by a normal build, rather than
/// only by `cargo test` and friends.
fn is_build_dependency(dep: &Value) -> bool {
    match dep.get("dep_kinds").and_then(Value::as_array) {
        Some(kinds) if !kinds.is_empty() => kinds
            .iter()
            .any(|kind| kind.get("kind").and_then(Value::as_str) != Some("dev")),
        _ => true,
    }
}

/// Count the resolved packages in the workspace `Cargo.lock` (each
/// `[[package]]` entry). Walks up from the current directory to find the lock
/// file and returns 0 when it can't be located, in which case the caller falls
/// back to an asymptotic estimate.
pub fn lock_package_count() -> usize {
    read_cargo_lock().map_or(0, |lock| {
        lock.lines().filter(|l| l.trim() == "[[package]]").count()
    })
}

/// Count the packages in `Cargo.lock` that come from a registry, the only ones
/// a build can download. Returns 0 when the lock file can't be located.
pub fn lock_registry_count() -> usize {
    read_cargo_lock().map_or(0, |lock| {
        lock.lines()
            .filter(|l| l.trim_start().starts_with("source = \"registry+"))
            .count()
    })
}

/// The contents of the nearest `Cargo.lock` that lists packages, searching
/// upward from the current directory.
fn read_cargo_lock() -> Option<String> {
    let mut dir = std::env::current_dir().ok();
    while let Some(d) = dir {
        if let Ok(contents) = std::fs::read_to_string(d.join("Cargo.lock"))
            && contents.contains("[[package]]")
        {
            return Some(contents);
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_package_counts_once_however_many_units_it_has() {
        let mut tracker = UnitTracker::default();
        assert_eq!(tracker.finish("dep", true), None);
        assert_eq!(tracker.finish("dep", false), Some(1));
        assert_eq!(tracker.finish("dep", false), None);
        assert_eq!(tracker.finish("other", false), Some(2));
    }

    #[test]
    fn units_without_a_package_id_all_count() {
        let mut tracker = UnitTracker::default();
        assert_eq!(tracker.finish("", false), Some(1));
        assert_eq!(tracker.finish("", false), Some(2));
    }

    #[test]
    fn expected_packages_follow_build_dependencies_only() {
        let metadata = json!({
            "workspace_members": ["app"],
            "resolve": {
                "root": "app",
                "nodes": [
                    {"id": "app", "deps": [
                        {"pkg": "lib", "dep_kinds": [{"kind": null}]},
                        {"pkg": "script", "dep_kinds": [{"kind": "build"}]},
                        {"pkg": "tester", "dep_kinds": [{"kind": "dev"}]}
                    ]},
                    {"id": "lib", "deps": [{"pkg": "leaf", "dep_kinds": [{"kind": null}]}]},
                    {"id": "script", "deps": []},
                    {"id": "leaf", "deps": []},
                    {"id": "tester", "deps": []},
                    {"id": "elsewhere", "deps": []}
                ]
            }
        });
        assert_eq!(reachable_packages(&metadata), Some(4));
    }

    #[test]
    fn a_virtual_workspace_starts_from_its_members() {
        let metadata = json!({
            "workspace_members": ["a", "b"],
            "resolve": {
                "root": null,
                "nodes": [
                    {"id": "a", "deps": [{"pkg": "shared", "dep_kinds": []}]},
                    {"id": "b", "deps": [{"pkg": "shared", "dep_kinds": []}]},
                    {"id": "shared", "deps": []}
                ]
            }
        });
        assert_eq!(reachable_packages(&metadata), Some(3));
    }
}
