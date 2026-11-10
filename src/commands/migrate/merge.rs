use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;

/// Files the scaffold owns outright: generated code and the Gradle wrapper.
///
/// They carry no project settings, and a stale copy is exactly what a migration
/// exists to fix (the iOS text bridge and the Android activity are revisioned
/// together with the Rust side), so they are always replaced. Every other file
/// in a scaffold may hold the user's own configuration — signing, bundle
/// identifiers, permissions, icons, build flags — and is kept once it exists.
const TOOL_OWNED: &[&str] = &[
    "main.swift",
    "AimerActivity.kt",
    "gradlew",
    "gradlew.bat",
    "gradle-wrapper.jar",
    "gradle-wrapper.properties",
];

/// What a merge did, with every path relative to the merged directory.
#[derive(Debug, Default)]
pub(super) struct Report {
    /// Files the project did not have yet.
    pub added: Vec<PathBuf>,
    /// Existing files replaced by the latest scaffold.
    pub refreshed: Vec<PathBuf>,
    /// Existing files that differ from the latest scaffold and were left alone.
    pub kept: Vec<PathBuf>,
}

/// Merge a freshly generated scaffold (`staged`) into the project's copy
/// (`target`) without deleting anything.
///
/// A file the project lacks is added, an identical one is skipped, and one that
/// differs is replaced only when it is tool-owned or `force` is set; otherwise
/// the user's version stays and is reported in [`Report::kept`]. Files that exist
/// only in `target` (build output, user additions) are never touched.
pub(super) fn merge(staged: &Path, target: &Path, force: bool) -> anyhow::Result<Report> {
    let mut files = Vec::new();
    collect(staged, staged, &mut files)?;
    files.sort();

    let mut report = Report::default();
    for rel in files {
        let from = staged.join(&rel);
        let to = target.join(&rel);

        if !to.exists() {
            install(&from, &to)?;
            report.added.push(rel);
        } else if same_content(&from, &to)? {
            continue;
        } else if force || is_tool_owned(&rel) {
            install(&from, &to)?;
            report.refreshed.push(rel);
        } else {
            report.kept.push(rel);
        }
    }
    Ok(report)
}

fn is_tool_owned(rel: &Path) -> bool {
    rel.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| TOOL_OWNED.contains(&name))
}

fn same_content(a: &Path, b: &Path) -> anyhow::Result<bool> {
    let read = |path: &Path| fs::read(path).with_context(|| format!("reading {}", path.display()));
    Ok(read(a)? == read(b)?)
}

/// Copy `from` to `to`, keeping its permissions (the Gradle wrapper script must
/// stay executable).
fn install(from: &Path, to: &Path) -> anyhow::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::copy(from, to).with_context(|| format!("writing {}", to.display()))?;
    Ok(())
}

/// Every file below `dir`, as a path relative to `root`.
fn collect(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_path_buf());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn read(root: &Path, rel: &str) -> String {
        fs::read_to_string(root.join(rel)).unwrap()
    }

    fn dirs() -> (tempfile::TempDir, tempfile::TempDir) {
        (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
    }

    #[test]
    fn missing_files_are_added() {
        let (staged, target) = dirs();
        write(staged.path(), "app/Info.plist", "new");

        let report = merge(staged.path(), target.path(), false).unwrap();

        assert_eq!(report.added, [PathBuf::from("app/Info.plist")]);
        assert_eq!(read(target.path(), "app/Info.plist"), "new");
    }

    #[test]
    fn user_edited_files_are_kept() {
        let (staged, target) = dirs();
        write(staged.path(), "app/Info.plist", "template");
        write(target.path(), "app/Info.plist", "my permissions");

        let report = merge(staged.path(), target.path(), false).unwrap();

        assert_eq!(report.kept, [PathBuf::from("app/Info.plist")]);
        assert!(report.added.is_empty() && report.refreshed.is_empty());
        assert_eq!(read(target.path(), "app/Info.plist"), "my permissions");
    }

    #[test]
    fn identical_files_are_not_reported() {
        let (staged, target) = dirs();
        write(staged.path(), "a.txt", "same");
        write(target.path(), "a.txt", "same");

        let report = merge(staged.path(), target.path(), false).unwrap();

        assert!(report.added.is_empty() && report.refreshed.is_empty() && report.kept.is_empty());
    }

    #[test]
    fn generated_code_is_refreshed() {
        let (staged, target) = dirs();
        write(staged.path(), "app/main.swift", "latest bridge");
        write(target.path(), "app/main.swift", "old bridge");

        let report = merge(staged.path(), target.path(), false).unwrap();

        assert_eq!(report.refreshed, [PathBuf::from("app/main.swift")]);
        assert_eq!(read(target.path(), "app/main.swift"), "latest bridge");
    }

    #[test]
    fn force_replaces_user_edited_files() {
        let (staged, target) = dirs();
        write(staged.path(), "app/Info.plist", "template");
        write(target.path(), "app/Info.plist", "my permissions");

        let report = merge(staged.path(), target.path(), true).unwrap();

        assert_eq!(report.refreshed, [PathBuf::from("app/Info.plist")]);
        assert_eq!(read(target.path(), "app/Info.plist"), "template");
    }

    #[test]
    fn files_only_the_project_has_are_untouched() {
        let (staged, target) = dirs();
        write(staged.path(), "a.txt", "a");
        write(target.path(), "pkg/artifact.wasm", "wasm");
        write(target.path(), "keystore.jks", "secret");

        merge(staged.path(), target.path(), true).unwrap();

        assert_eq!(read(target.path(), "pkg/artifact.wasm"), "wasm");
        assert_eq!(read(target.path(), "keystore.jks"), "secret");
    }

    #[cfg(unix)]
    #[test]
    fn installed_files_keep_their_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (staged, target) = dirs();
        write(staged.path(), "gradlew", "#!/bin/sh");
        fs::set_permissions(
            staged.path().join("gradlew"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        merge(staged.path(), target.path(), false).unwrap();

        let mode = fs::metadata(target.path().join("gradlew"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
    }
}
