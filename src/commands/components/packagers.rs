//! The installer generators `aimer package` runs: `appimagetool` (Linux
//! `.AppImage`), Inno Setup (Windows `setup.exe`) and WiX (Windows `.msi`).
//!
//! * `appimagetool` is downloaded from its GitHub release into `~/.aimer/bin`.
//! * Inno Setup is installed silently into `~/.aimer/innosetup`; `ISCC.exe`
//!   needs the files beside it, so it cannot live in `bin`.
//! * WiX is a .NET tool; `dotnet tool install --tool-path` puts it in `bin`
//!   (see `dotnet_install_command` in the parent module).
//!
//! `aimer package` finds all three there, so packaging needs no setup.

#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

use super::{bin_dir, binary_path};

const APPIMAGETOOL_RELEASE: &str =
    "https://github.com/AppImage/appimagetool/releases/download/continuous";
const INNO_SETUP_URL: &str = "https://jrsoftware.org/download.php/is.exe";

/// The file name of the `appimagetool` AppImage for the CPU architecture
/// `arch` (Rust's name for it).
pub(super) fn appimagetool_url(arch: &str) -> anyhow::Result<String> {
    let arch = match arch {
        "x86_64" | "aarch64" => arch,
        "x86" => "i686",
        "arm" => "armhf",
        other => bail!("appimagetool has no download for the '{other}' architecture"),
    };
    Ok(format!(
        "{APPIMAGETOOL_RELEASE}/appimagetool-{arch}.AppImage"
    ))
}

/// Where Inno Setup is installed inside the install `root`.
pub(super) fn inno_dir(root: &Path) -> PathBuf {
    root.join("innosetup")
}

/// Inno Setup's command-line compiler, inside the install `root`.
pub(super) fn iscc_path(root: &Path) -> PathBuf {
    inno_dir(root).join("ISCC.exe")
}

/// The command that installs Inno Setup silently, for the current user only,
/// into `dir`.
pub(super) fn inno_install_command(installer: &Path, dir: &Path) -> Command {
    let mut dir_arg = OsString::from("/DIR=");
    dir_arg.push(dir);
    let mut cmd = Command::new(installer);
    cmd.args([
        "/VERYSILENT",
        "/SUPPRESSMSGBOXES",
        "/NORESTART",
        "/CURRENTUSER",
    ])
    .arg(dir_arg);
    cmd
}

/// The command that runs Inno Setup's own uninstaller silently.
fn inno_uninstall_command(root: &Path) -> Command {
    let mut cmd = Command::new(inno_dir(root).join("unins000.exe"));
    cmd.args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]);
    cmd
}

/// Download `url` into `dest`.
fn fetch(url: &str, dest: &Path) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    let bytes = runtime.block_on(async {
        reqwest::Client::builder()
            .user_agent(concat!("aimer/", env!("CARGO_PKG_VERSION")))
            .build()?
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await
    });
    let bytes = bytes.with_context(|| format!("downloading {url}"))?;
    std::fs::write(dest, bytes).with_context(|| format!("writing {}", dest.display()))
}

/// Download `appimagetool` into the install `root`'s `bin` directory.
pub(super) fn install_appimagetool(root: &Path) -> anyhow::Result<()> {
    let url = appimagetool_url(std::env::consts::ARCH)?;
    let bin = bin_dir(root);
    std::fs::create_dir_all(&bin).with_context(|| format!("creating {}", bin.display()))?;
    let target = binary_path(root, "appimagetool");
    // Written beside the target so the last step is a rename.
    let partial = bin.join(".appimagetool.download");
    println!("Downloading {url}");
    fetch(&url, &partial)?;
    make_executable(&partial)?;
    std::fs::rename(&partial, &target)
        .with_context(|| format!("moving appimagetool to {}", target.display()))
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

/// Download Inno Setup's installer and run it silently into the install
/// `root`.
pub(super) fn install_inno_setup(root: &Path) -> anyhow::Result<()> {
    let staging = tempfile::tempdir().context("creating a download directory")?;
    let installer = staging.path().join("innosetup.exe");
    println!("Downloading {INNO_SETUP_URL}");
    fetch(INNO_SETUP_URL, &installer)?;
    let dir = inno_dir(root);
    println!("Installing Inno Setup into {}", dir.display());
    let status = inno_install_command(&installer, &dir)
        .status()
        .context("running the Inno Setup installer")?;
    if !status.success() || !iscc_path(root).is_file() {
        bail!("the Inno Setup installer did not install ISCC.exe");
    }
    Ok(())
}

/// Uninstall Inno Setup from the install `root`. `Ok(false)` means it was not
/// installed.
pub(super) fn remove_inno_setup(root: &Path) -> anyhow::Result<bool> {
    let dir = inno_dir(root);
    if !dir.exists() {
        return Ok(false);
    }
    // Let Inno's uninstaller clean up its registry entries; the folder is
    // removed either way.
    if dir.join("unins000.exe").is_file() {
        let _ = inno_uninstall_command(root).status();
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
    }
    Ok(true)
}
