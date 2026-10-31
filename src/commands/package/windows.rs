//! Packaging a Windows app into a setup `.exe`, an `.msi` or a `.zip`.
//!
//! All three start from the portable bundle `aimer assemble windows` makes: a
//! folder holding the executable, the registered assets, the icon (`app.ico`)
//! and the manifest. The runtime looks assets up next to the executable, so
//! every package installs that folder as it is.
//!
//! * A **setup `.exe`** is built by Inno Setup's `ISCC` from a generated script
//!   ([`inno`]). It installs per user by default, without administrator rights,
//!   and offers an all-users install.
//! * An **`.msi`** is built by the WiX Toolset (v4 or later, `wix build`) from a
//!   generated source ([`wix`]). It installs per machine, the form IT
//!   departments deploy.
//! * A **`.zip`** is the bundle, zipped; unzip it anywhere and run the
//!   executable.
//!
//! Neither tool is downloaded for you: install it and put it on `PATH`, or
//! point `AIMER_ISCC` / `AIMER_WIX` at it. Both are looked for before the build
//! starts. Packages are not signed yet, so Windows SmartScreen warns about an
//! installer from an unknown publisher.

mod inno;
mod wix;

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};

use super::archive::create_zip;
use super::tools::{self, ToolSpec};
use super::{PackageFormat, Request, dist_dir, package_stem, remove_stale, reset_dir};
use crate::commands::assemble::{self, Reporter, StdioReporter, Step, StepKind, profile_name};
use crate::config::{AimerManifest, resolve_cargo_package_name};
use crate::targets::Targets;

const ENV_ISCC: &str = "AIMER_ISCC";
const ENV_WIX: &str = "AIMER_WIX";

/// Scratch space of one packaging run: the generated script or source.
const WORK_DIR_NAME: &str = ".windows-work";

/// What to package and where the result goes.
pub(super) struct WindowsJob<'a> {
    pub format: PackageFormat,
    /// The Cargo package, which names the executable.
    pub pkg_name: &'a str,
    /// The application name, which labels the package and the shortcut.
    pub app_name: &'a str,
    pub version: &'a str,
    /// `[package] author`, shown as the publisher.
    pub publisher: &'a str,
    /// `[package] group`, from which the installer's stable id derives.
    pub group: &'a str,
    /// The portable bundle, as an absolute path.
    pub bundle: &'a Path,
    /// Directory the package is written to.
    pub dist: &'a str,
    /// `ISCC` or `wix`; not needed for a `.zip`.
    pub tool: Option<&'a Path>,
}

/// Build the Windows app and package it as the request says; returns the
/// package.
pub(super) fn run(request: &Request, manifest: Option<&AimerManifest>) -> anyhow::Result<String> {
    let release = request.release;
    let package = manifest.map(|m| &m.package);
    let version = package.map(|p| p.version.clone()).unwrap_or_default();

    // Checked before the (slow) build, so a missing tool or a version an MSI
    // cannot carry fails in seconds.
    let env = |key: &str| std::env::var(key).ok();
    let tool = match request.format {
        PackageFormat::Exe => Some(locate_iscc(&env, &|path| path.is_file())?),
        PackageFormat::Msi => {
            wix::msi_version(&version)?;
            Some(locate_wix(&env, &|path| path.is_file())?)
        }
        _ => None,
    };

    println!(
        "Packaging 'windows' app into a .{} in {} mode...",
        request.format.extension(),
        profile_name(release)
    );

    let reporter = StdioReporter;
    let pkg_name = resolve_cargo_package_name(Path::new("."));
    assemble::build_desktop(Targets::Windows, &pkg_name, release, &reporter)?;
    let bundle = assemble::package_desktop(Targets::Windows, &pkg_name, release, &reporter)?;

    let app_name = assemble::app_name();
    let dist = dist_dir(request.target, release);
    let bundle = std::path::absolute(&bundle).context("resolving the bundle")?;
    let job = WindowsJob {
        format: request.format,
        pkg_name: &pkg_name,
        app_name: &app_name,
        version: &version,
        publisher: package.map(|p| p.author.as_str()).unwrap_or_default(),
        group: package.map(|p| p.group.as_str()).unwrap_or_default(),
        bundle: &bundle,
        dist: &dist,
        tool: tool.as_deref(),
    };
    package_bundle(&job, &reporter)
}

/// `ISCC`, Inno Setup's command-line compiler: `AIMER_ISCC`, one on `PATH`, or
/// the folder Inno Setup installs itself to.
///
/// `env` and `is_file` are injected so the search can be tested without
/// touching the machine.
///
/// # Errors
///
/// Fails, saying where to get Inno Setup, when none is found.
pub(super) fn locate_iscc(
    env: &dyn Fn(&str) -> Option<String>,
    is_file: &dyn Fn(&Path) -> bool,
) -> anyhow::Result<PathBuf> {
    let under = |var: &str, rest: &str| env(var).map(|base| Path::new(&base).join(rest));
    let extra_dirs = [
        under("ProgramFiles(x86)", "Inno Setup 6"),
        under("ProgramFiles", "Inno Setup 6"),
        under("LOCALAPPDATA", "Programs/Inno Setup 6"),
    ]
    .into_iter()
    .flatten()
    .collect();
    tools::locate(
        &ToolSpec {
            name: "ISCC (Inno Setup)",
            env_var: ENV_ISCC,
            file_names: &["ISCC.exe", "iscc.exe", "ISCC", "iscc"],
            extra_dirs,
            how_to_get: "install Inno Setup 6.3 or later from https://jrsoftware.org/isdl.php \
                         and put its folder on PATH",
        },
        env,
        is_file,
    )
}

/// `wix`, the WiX Toolset v4+ command line: `AIMER_WIX`, one on `PATH`, or the
/// folder `dotnet tool install --global` puts it in.
///
/// # Errors
///
/// Fails, saying how to install WiX, when none is found.
pub(super) fn locate_wix(
    env: &dyn Fn(&str) -> Option<String>,
    is_file: &dyn Fn(&Path) -> bool,
) -> anyhow::Result<PathBuf> {
    let extra_dirs = ["USERPROFILE", "HOME"]
        .into_iter()
        .filter_map(|var| env(var))
        .map(|home| Path::new(&home).join(".dotnet/tools"))
        .collect();
    tools::locate(
        &ToolSpec {
            name: "wix (WiX Toolset)",
            env_var: ENV_WIX,
            file_names: &["wix.exe", "wix"],
            extra_dirs,
            how_to_get: "install the WiX Toolset v4 or later with \
                         `dotnet tool install --global wix` (WiX 3's candle and light are not \
                         supported)",
        },
        env,
        is_file,
    )
}

/// The architecture name Windows packages use.
pub(super) fn windows_arch() -> &'static str {
    arch_name(std::env::consts::ARCH)
}

/// Rust's name for an architecture, as Windows installers spell it.
pub(super) fn arch_name(rust_arch: &'static str) -> &'static str {
    match rust_arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }
}

/// The file name of a package, without its directory:
/// `<app>-<version>-<arch>-setup.exe`, `<app>-<version>-<arch>.msi` or
/// `<app>-<version>-windows-<arch>.zip`.
pub(super) fn output_name(
    format: PackageFormat,
    app_name: &str,
    version: &str,
    arch: &str,
) -> String {
    let stem = package_stem(app_name, version);
    match format {
        PackageFormat::Exe => format!("{stem}-{arch}-setup.exe"),
        PackageFormat::Zip => format!("{stem}-windows-{arch}.zip"),
        _ => format!("{stem}-{arch}.{}", format.extension()),
    }
}

/// A GUID that depends only on `seed`, so that every build of the same app
/// carries the same id: it is what lets an installer recognize and upgrade an
/// earlier version instead of installing beside it.
pub(super) fn stable_guid(seed: &str) -> String {
    let hash = Sha256::digest(seed.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    // Version 5 (name based) and the RFC 4122 variant.
    bytes[6] = (bytes[6] & 0x0F) | 0x50;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The seed of an app's installer ids: its bundle id when it has one, else its
/// package name, so two apps never share an id.
fn id_seed(job: &WindowsJob, kind: &str) -> String {
    let base = if job.group.trim().is_empty() {
        job.pkg_name
    } else {
        job.group.trim()
    };
    format!("aimer-{kind}:{base}")
}

/// Package the bundle of `job`; returns the path of the package.
pub(super) fn package_bundle(job: &WindowsJob, reporter: &dyn Reporter) -> anyhow::Result<String> {
    std::fs::create_dir_all(job.dist).with_context(|| format!("creating {}", job.dist))?;
    let work = std::path::absolute(format!("{}/{WORK_DIR_NAME}", job.dist))
        .context("resolving the scratch directory")?;
    reset_dir(&work.to_string_lossy())?;

    let name = output_name(job.format, job.app_name, job.version, windows_arch());
    let output = format!("{}/{name}", job.dist);
    remove_stale(&output)?;
    let output_abs = std::path::absolute(&output).context("resolving the package path")?;

    let result = match job.format {
        PackageFormat::Zip => zip_bundle(job, &output_abs, reporter),
        PackageFormat::Exe => setup_exe(job, &work, &name, reporter),
        _ => msi(job, &work, &output_abs, reporter),
    };
    let _ = std::fs::remove_dir_all(&work);
    result?;

    if !Path::new(&output).is_file() {
        bail!("the packaging tool finished but {output} was not produced");
    }
    Ok(output)
}

fn zip_bundle(job: &WindowsJob, output: &Path, reporter: &dyn Reporter) -> anyhow::Result<()> {
    reporter.note(format!("Creating {}...", output.display()));
    create_zip(job.bundle, &package_stem(job.pkg_name, job.version), output)
}

fn setup_exe(
    job: &WindowsJob,
    work: &Path,
    name: &str,
    reporter: &dyn Reporter,
) -> anyhow::Result<()> {
    let Some(tool) = job.tool else {
        bail!("no ISCC to build the setup program with");
    };
    let dist = std::path::absolute(job.dist).context("resolving the package folder")?;
    let script = inno::script(&inno::Installer {
        app_name: job.app_name,
        exe_name: &format!("{}.exe", job.pkg_name),
        version: job.version,
        publisher: job.publisher,
        bundle: job.bundle,
        dist: &dist,
        base_name: name.strip_suffix(".exe").unwrap_or(name),
        app_id: &stable_guid(&id_seed(job, "inno")),
        arch: windows_arch(),
    });
    let path = work.join("setup.iss");
    std::fs::write(&path, script).with_context(|| format!("writing {}", path.display()))?;

    reporter.note(format!("Building the setup program {name}..."));
    reporter.run(
        inno::command(tool, &path),
        Step::new(StepKind::Other, "ISCC"),
    )
}

fn msi(
    job: &WindowsJob,
    work: &Path,
    output: &Path,
    reporter: &dyn Reporter,
) -> anyhow::Result<()> {
    let Some(tool) = job.tool else {
        bail!("no wix to build the MSI with");
    };
    let source = wix::source(&wix::Installer {
        app_name: job.app_name,
        exe_name: &format!("{}.exe", job.pkg_name),
        version: job.version,
        publisher: job.publisher,
        bundle: job.bundle,
        upgrade_code: &stable_guid(&id_seed(job, "wix")),
    })?;
    let path = work.join("package.wxs");
    std::fs::write(&path, source).with_context(|| format!("writing {}", path.display()))?;

    reporter.note(format!("Building {}...", output.display()));
    reporter.run(
        wix::command(tool, &path, output, windows_arch()),
        Step::new(StepKind::Other, "wix build"),
    )
}

#[cfg(test)]
mod tests;
