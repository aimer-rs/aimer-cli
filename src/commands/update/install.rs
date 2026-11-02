//! Unpacking a downloaded release and swapping it in for the running binary.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

/// The command that unpacks `archive` into `dest`. Windows' `tar` reads zips.
pub(super) fn extract_command(archive: &Path, dest: &Path) -> Command {
    let zip = archive.extension().is_some_and(|ext| ext == "zip");
    let mut cmd = Command::new("tar");
    cmd.arg(if zip { "-xf" } else { "-xzf" })
        .arg(archive)
        .arg("-C")
        .arg(dest);
    cmd
}

/// Unpack `archive` into `dest`.
pub(super) fn extract(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let status = extract_command(archive, dest)
        .status()
        .context("running `tar` (is it installed?)")?;
    if !status.success() {
        bail!("`tar` failed to unpack {}", archive.display());
    }
    Ok(())
}

/// The executable's file name on this OS.
pub(super) fn binary_name() -> &'static str {
    if cfg!(windows) { "aimer.exe" } else { "aimer" }
}

/// The new binary inside the unpacked release: `<dir>/<folder>/aimer`, or
/// `<dir>/aimer` if the archive has no folder.
pub(super) fn find_binary(dir: &Path, folder: &str) -> Option<PathBuf> {
    let name = binary_name();
    [dir.join(folder).join(name), dir.join(name)]
        .into_iter()
        .find(|path| path.is_file())
}

/// Make sure `binary` starts, so a broken download never replaces a working
/// install.
pub(super) fn verify_runs(binary: &Path) -> anyhow::Result<()> {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .with_context(|| format!("running {}", binary.display()))?;
    if !output.status.success() {
        bail!("the downloaded aimer does not run on this machine");
    }
    Ok(())
}

/// The path of the running binary, with symlinks resolved so the real file is
/// the one replaced.
pub(super) fn current_exe() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe().context("finding the running aimer")?;
    Ok(exe.canonicalize().unwrap_or(exe))
}

/// Refuse to overwrite a binary that `cargo` built in a `target` directory.
pub(super) fn ensure_installed_build(exe: &Path) -> anyhow::Result<()> {
    let profile = exe.parent();
    let in_target = profile
        .and_then(Path::file_name)
        .is_some_and(|name| name == "debug" || name == "release")
        && profile
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|name| name == "target");
    if in_target {
        bail!(
            "this aimer ({}) was built from source; rebuild it with cargo instead",
            exe.display()
        );
    }
    Ok(())
}

/// Replace `current` with `new`.
pub(super) fn replace(current: &Path, new: &Path) -> anyhow::Result<()> {
    replace_impl(current, new, cfg!(windows))
}

/// `move_aside`: a running executable cannot be overwritten on Windows, but
/// it can be renamed, so the old file is renamed to `<name>.old` first.
pub(super) fn replace_impl(current: &Path, new: &Path, move_aside: bool) -> anyhow::Result<()> {
    let dir = current.parent().context("the aimer binary has no folder")?;
    let name = current
        .file_name()
        .context("the aimer binary has no name")?
        .to_string_lossy()
        .into_owned();
    // Staged beside the target so the final step is a same-volume rename.
    let staged = dir.join(format!(".{name}.new"));
    std::fs::copy(new, &staged).map_err(|err| write_error(err, dir))?;
    make_executable(&staged)?;

    let result = if move_aside {
        let old = dir.join(format!("{name}.old"));
        let _ = std::fs::remove_file(&old);
        std::fs::rename(current, &old)
            .and_then(|()| {
                std::fs::rename(&staged, current).inspect_err(|_| {
                    let _ = std::fs::rename(&old, current);
                })
            })
            .map_err(|err| write_error(err, dir))
    } else {
        std::fs::rename(&staged, current).map_err(|err| write_error(err, dir))
    };
    if result.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    result
}

fn write_error(err: std::io::Error, dir: &Path) -> anyhow::Error {
    if err.kind() == std::io::ErrorKind::PermissionDenied {
        anyhow::anyhow!(
            "no permission to write to {}; run the update with the rights needed to modify it \
             ({err})",
            dir.display()
        )
    } else {
        anyhow::Error::new(err).context(format!("replacing the binary in {}", dir.display()))
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .with_context(|| format!("making {} executable", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}
