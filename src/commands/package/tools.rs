//! Finding the external tool a package format needs (`appimagetool`, Inno
//! Setup's `ISCC`, WiX's `wix`).
//!
//! These tools are never downloaded: fetching and running a binary behind the
//! user's back is not something a packaging command should do. They are looked
//! for in the order a user would expect, and when none is found the error says
//! where to get one and how to point `aimer` at it.

use std::path::{Path, PathBuf};

use anyhow::bail;

/// What to look for, and how to describe it when it is missing.
pub(super) struct ToolSpec<'a> {
    /// The tool's name, for messages.
    pub name: &'a str,
    /// The environment variable that names the tool's path.
    pub env_var: &'a str,
    /// The file names it may have on `PATH` or in `extra_dirs`.
    pub file_names: &'a [&'a str],
    /// Folders it is usually installed to without being put on `PATH`.
    pub extra_dirs: Vec<PathBuf>,
    /// How to get it, as the start of a sentence's remedy: "download it from
    /// ...".
    pub how_to_get: &'a str,
}

/// The tool described by `spec`: the path in its environment variable, or the
/// first match on `PATH`, then in `extra_dirs`.
///
/// `env` and `is_file` are injected so the search can be tested without
/// touching the machine.
///
/// # Errors
///
/// Fails when the environment variable names something that is not a file
/// (a typo must not silently fall back to a different tool), or when nothing
/// is found.
pub(super) fn locate(
    spec: &ToolSpec,
    env: &dyn Fn(&str) -> Option<String>,
    is_file: &dyn Fn(&Path) -> bool,
) -> anyhow::Result<PathBuf> {
    if let Some(path) = env(spec.env_var).filter(|p| !p.trim().is_empty()) {
        let path = PathBuf::from(path.trim());
        if is_file(&path) {
            return Ok(path);
        }
        bail!(
            "{} is set to '{}', which is not a file",
            spec.env_var,
            path.display()
        );
    }

    let on_path = env("PATH")
        .map(|search| std::env::split_paths(&search).collect::<Vec<_>>())
        .unwrap_or_default();
    for dir in on_path.iter().chain(spec.extra_dirs.iter()) {
        for name in spec.file_names {
            let candidate = dir.join(name);
            if is_file(&candidate) {
                return Ok(candidate);
            }
        }
    }

    bail!(
        "{} was not found: {}, or set {} to its path",
        spec.name,
        spec.how_to_get,
        spec.env_var
    )
}

#[cfg(test)]
mod tests;
