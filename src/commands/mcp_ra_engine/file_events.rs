//! Decide what a workspace file-watcher event means for the embedded engine.
//!
//! Native watchers also report reads (`Access` events) and metadata-only
//! changes. Those never change source text, and treating them as edits made
//! every `search_text` call, editor, or build that read the tree force a
//! workspace reload or a full re-index on the next Rust request.

use std::path::{Component, Path};

use notify::EventKind;
use notify::event::ModifyKind;

/// What the engine should do about one changed path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FileEventAction {
    /// The event cannot have changed anything the engine depends on.
    Ignore,
    /// Re-read this indexed Rust file and apply its text if it changed.
    UpdateSource,
    /// The crate graph or file set may have changed; reload the workspace.
    Reload,
}

/// Classify `path` from an event of `kind` inside the workspace `root`.
pub(super) fn classify(kind: &EventKind, path: &Path, root: &Path) -> FileEventAction {
    if !changes_contents(kind) || is_generated_or_vcs(path, root) {
        return FileEventAction::Ignore;
    }
    if is_manifest_or_build_script(path) {
        return FileEventAction::Reload;
    }
    if path.extension().is_none_or(|extension| extension != "rs") {
        return FileEventAction::Ignore;
    }
    let structural = matches!(
        kind,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
    );
    if structural || !path.is_file() {
        FileEventAction::Reload
    } else {
        FileEventAction::UpdateSource
    }
}

/// Reads and metadata-only changes (permissions, timestamps) leave file
/// contents untouched.
fn changes_contents(kind: &EventKind) -> bool {
    !matches!(
        kind,
        EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(_))
    )
}

fn is_generated_or_vcs(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root).ok().is_some_and(|relative| {
        relative.components().any(|component| {
            matches!(component, Component::Normal(name) if name == "target" || name == ".git" || name == ".aimer")
        })
    })
}

/// `Cargo.toml` anywhere, and `build.rs` only where Cargo would run it: next
/// to a `Cargo.toml`. A module that happens to be named `build.rs` (such as
/// `src/commands/build.rs`) is ordinary source.
fn is_manifest_or_build_script(path: &Path) -> bool {
    match path.file_name().and_then(|name| name.to_str()) {
        Some("Cargo.toml") => true,
        Some("build.rs") => path
            .parent()
            .is_some_and(|directory| directory.join("Cargo.toml").is_file()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, AccessMode, CreateKind, DataChange, MetadataKind, RenameMode};
    use std::fs;

    fn workspace() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        fs::write(root.path().join("build.rs"), "fn main() {}\n").unwrap();
        fs::create_dir_all(root.path().join("src/commands")).unwrap();
        fs::write(root.path().join("src/lib.rs"), "pub mod commands;\n").unwrap();
        fs::write(
            root.path().join("src/commands/build.rs"),
            "pub fn run() {}\n",
        )
        .unwrap();
        root
    }

    const MODIFY_DATA: EventKind = EventKind::Modify(ModifyKind::Data(DataChange::Any));

    #[test]
    fn reads_never_trigger_work() {
        let root = workspace();
        for kind in [
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
        ] {
            for file in ["Cargo.toml", "build.rs", "src/lib.rs"] {
                assert_eq!(
                    classify(&kind, &root.path().join(file), root.path()),
                    FileEventAction::Ignore,
                    "{kind:?} on {file}"
                );
            }
        }
    }

    #[test]
    fn manifest_and_build_script_edits_reload() {
        let root = workspace();
        for file in ["Cargo.toml", "build.rs"] {
            assert_eq!(
                classify(&MODIFY_DATA, &root.path().join(file), root.path()),
                FileEventAction::Reload
            );
        }
    }

    #[test]
    fn a_module_named_build_rs_is_ordinary_source() {
        let root = workspace();
        let module = root.path().join("src/commands/build.rs");
        assert_eq!(
            classify(&MODIFY_DATA, &module, root.path()),
            FileEventAction::UpdateSource
        );
    }

    #[test]
    fn source_edits_update_and_structural_changes_reload() {
        let root = workspace();
        let source = root.path().join("src/lib.rs");
        assert_eq!(
            classify(&MODIFY_DATA, &source, root.path()),
            FileEventAction::UpdateSource
        );
        for kind in [
            EventKind::Create(CreateKind::File),
            EventKind::Modify(ModifyKind::Name(RenameMode::To)),
        ] {
            assert_eq!(
                classify(&kind, &source, root.path()),
                FileEventAction::Reload
            );
        }
    }

    #[test]
    fn target_git_and_non_rust_files_are_ignored() {
        let root = workspace();
        for file in ["target/debug/build/x/out/gen.rs", ".git/index", "README.md"] {
            assert_eq!(
                classify(&MODIFY_DATA, &root.path().join(file), root.path()),
                FileEventAction::Ignore,
                "{file}"
            );
        }
    }
}
