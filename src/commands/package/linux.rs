//! Packaging a Linux app into an `.AppImage` or a `.tar.gz`.
//!
//! Both start from the portable bundle `aimer assemble linux` makes: a folder
//! holding the executable, the registered assets, the icon (`app.png`) and the
//! desktop entry. The runtime looks assets up next to the executable, so that
//! layout is kept intact inside either package.
//!
//! * A **`.tar.gz`** is the bundle under a clean top-level folder, compressed
//!   with `tar`.
//! * An **`.AppImage`** is an `AppDir`, which `appimagetool` turns into a
//!   single runnable file. The bundle goes to `usr/bin/`, next to which sit the
//!   `AppRun` launcher, the desktop entry and the icon that the format
//!   requires at the top. `appimagetool` is not downloaded for you: install it
//!   and put it on `PATH`, or point `AIMER_APPIMAGETOOL` at it.
//!
//! All external tools are built as plain [`Command`]s and handed to the
//! [`Reporter`], so the argument lists can be asserted without touching the
//! machine.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

use super::{PackageFormat, Request, dist_dir, package_stem, remove_stale, reset_dir};
use crate::commands::assemble::{self, Reporter, StdioReporter, Step, StepKind, profile_name};
use crate::config::{AimerManifest, resolve_cargo_package_name};
use crate::targets::Targets;

/// Points at an `appimagetool` that is not on `PATH`.
const ENV_APPIMAGETOOL: &str = "AIMER_APPIMAGETOOL";

/// Where to get `appimagetool`, for the error that says it is missing.
const APPIMAGETOOL_URL: &str = "https://github.com/AppImage/appimagetool/releases";

/// Scratch space of one packaging run: the `AppDir` or the staged folder.
const WORK_DIR_NAME: &str = ".linux-work";

/// What to package and where the result goes.
pub(super) struct LinuxJob<'a> {
    pub format: PackageFormat,
    /// The Cargo package, which names the executable.
    pub pkg_name: &'a str,
    /// The application name, which labels the package.
    pub app_name: &'a str,
    pub version: &'a str,
    /// The portable bundle, as an absolute path.
    pub bundle: &'a Path,
    /// Directory the package is written to.
    pub dist: &'a str,
    /// The `appimagetool` to use; only needed for an AppImage.
    pub appimagetool: Option<&'a Path>,
}

/// Build the Linux app and package it as the request says; returns the package.
pub(super) fn run(request: &Request, manifest: Option<&AimerManifest>) -> anyhow::Result<String> {
    let release = request.release;
    // Looked for before the (slow) build, so a missing tool fails in seconds.
    let appimagetool = if request.format == PackageFormat::Appimage {
        Some(locate_appimagetool(
            &|key| std::env::var(key).ok(),
            &|path| path.is_file(),
        )?)
    } else {
        None
    };

    println!(
        "Packaging 'linux' app into a .{} in {} mode...",
        request.format.extension(),
        profile_name(release)
    );

    let reporter = StdioReporter;
    let pkg_name = resolve_cargo_package_name(Path::new("."));
    assemble::build_desktop(Targets::Linux, &pkg_name, release, &reporter)?;
    let bundle = assemble::package_desktop(Targets::Linux, &pkg_name, release, &reporter)?;

    let version = manifest
        .map(|m| m.package.version.clone())
        .unwrap_or_default();
    let app_name = assemble::app_name();
    let dist = dist_dir(request.target, release);
    let bundle = std::path::absolute(&bundle).context("resolving the bundle")?;
    let job = LinuxJob {
        format: request.format,
        pkg_name: &pkg_name,
        app_name: &app_name,
        version: &version,
        bundle: &bundle,
        dist: &dist,
        appimagetool: appimagetool.as_deref(),
    };
    package_bundle(&job, &reporter)
}

/// The `appimagetool` to run: `AIMER_APPIMAGETOOL`, or one on `PATH`.
///
/// `env` and `is_file` are injected so the search can be tested without
/// touching the machine.
///
/// # Errors
///
/// Fails, saying where to get the tool, when none is found.
pub(super) fn locate_appimagetool(
    env: &dyn Fn(&str) -> Option<String>,
    is_file: &dyn Fn(&Path) -> bool,
) -> anyhow::Result<PathBuf> {
    if let Some(path) = env(ENV_APPIMAGETOOL).filter(|p| !p.trim().is_empty()) {
        let path = PathBuf::from(path.trim());
        if is_file(&path) {
            return Ok(path);
        }
        bail!(
            "{ENV_APPIMAGETOOL} is set to '{}', which is not a file",
            path.display()
        );
    }
    // The release page ships it as `appimagetool-<arch>.AppImage`.
    let names = [
        "appimagetool".to_string(),
        format!("appimagetool-{}.AppImage", appimage_arch()),
    ];
    if let Some(search) = env("PATH") {
        for dir in std::env::split_paths(&search) {
            for name in &names {
                let candidate = dir.join(name);
                if is_file(&candidate) {
                    return Ok(candidate);
                }
            }
        }
    }
    bail!(
        "appimagetool was not found: download it from {APPIMAGETOOL_URL}, make it executable and \
         put it on PATH as `appimagetool`, or set {ENV_APPIMAGETOOL} to its path"
    )
}

/// The architecture name `appimagetool` and the file names use.
pub(super) fn appimage_arch() -> &'static str {
    arch_name(std::env::consts::ARCH)
}

/// Rust's name for an architecture, as `appimagetool` spells it.
pub(super) fn arch_name(rust_arch: &'static str) -> &'static str {
    match rust_arch {
        "x86" => "i686",
        "arm" => "armhf",
        other => other,
    }
}

/// The file name of a package, without its directory.
///
/// An AppImage is named `<app>-<version>-<arch>.AppImage`, the usual
/// convention; an archive adds `linux`, since it is not self-describing:
/// `<app>-<version>-linux-<arch>.tar.gz`.
pub(super) fn output_name(
    format: PackageFormat,
    app_name: &str,
    version: &str,
    arch: &str,
) -> String {
    let stem = package_stem(app_name, version);
    match format {
        PackageFormat::TarGz => format!("{stem}-linux-{arch}.{}", format.extension()),
        _ => format!("{stem}-{arch}.{}", format.extension()),
    }
}

/// Package the bundle of `job`; returns the path of the package.
pub(super) fn package_bundle(job: &LinuxJob, reporter: &dyn Reporter) -> anyhow::Result<String> {
    std::fs::create_dir_all(job.dist).with_context(|| format!("creating {}", job.dist))?;
    let work = std::path::absolute(format!("{}/{WORK_DIR_NAME}", job.dist))
        .context("resolving the scratch directory")?;
    reset_dir(&work.to_string_lossy())?;

    let output = format!(
        "{}/{}",
        job.dist,
        output_name(job.format, job.app_name, job.version, appimage_arch())
    );
    remove_stale(&output)?;
    let output_abs = std::path::absolute(&output).context("resolving the package path")?;

    let result = match job.format {
        PackageFormat::TarGz => tar_gz(job, &work, &output_abs, reporter),
        _ => app_image(job, &work, &output_abs, reporter),
    };
    // The scratch folder is a full copy of the bundle; never leave it behind.
    let _ = std::fs::remove_dir_all(&work);
    result?;

    if !Path::new(&output).is_file() {
        bail!("the packaging tool finished but {output} was not produced");
    }
    Ok(output)
}

fn tar_gz(
    job: &LinuxJob,
    work: &Path,
    output: &Path,
    reporter: &dyn Reporter,
) -> anyhow::Result<()> {
    // The archive opens into one folder with a clean name, not the bundle's
    // build-layout one.
    let top = package_stem(job.pkg_name, job.version);
    copy_dir(job.bundle, &work.join(&top))?;

    reporter.note(format!("Creating {}...", output.display()));
    reporter.run(
        tar_command(work, &top, output),
        Step::new(StepKind::Other, "tar"),
    )
}

fn app_image(
    job: &LinuxJob,
    work: &Path,
    output: &Path,
    reporter: &dyn Reporter,
) -> anyhow::Result<()> {
    let Some(tool) = job.appimagetool else {
        bail!("no appimagetool to build the AppImage with");
    };
    let app_dir = work.join("AppDir");
    build_app_dir(job.bundle, &app_dir, job.pkg_name, job.app_name)?;

    reporter.note(format!("Creating {}...", output.display()));
    reporter.run(
        appimagetool_command(tool, &app_dir, output),
        Step::new(StepKind::Other, "appimagetool"),
    )
}

/// Lay out the `AppDir` that `appimagetool` turns into an AppImage.
///
/// ```text
/// AppDir/
///   AppRun                 launcher, runs usr/bin/<pkg>
///   <pkg>.desktop          the desktop entry, pointing at <pkg>
///   <pkg>.png, .DirIcon    the icon
///   usr/bin/               the whole bundle: executable, assets, ...
/// ```
///
/// # Errors
///
/// Fails when the bundle has no executable or no icon, since an AppImage
/// without either would not build or would show a blank icon.
pub(super) fn build_app_dir(
    bundle: &Path,
    app_dir: &Path,
    pkg_name: &str,
    app_name: &str,
) -> anyhow::Result<()> {
    if !bundle.join(pkg_name).is_file() {
        bail!(
            "the bundle {} has no executable '{pkg_name}'",
            bundle.display()
        );
    }
    let icon = bundle.join("app.png");
    if !icon.is_file() {
        bail!("the bundle has no icon: builds/linux/app.png is missing, and an AppImage needs one");
    }

    // Assets are found next to the executable, so the whole bundle moves
    // together.
    copy_dir(bundle, &app_dir.join("usr/bin"))?;

    let run = app_dir.join("AppRun");
    std::fs::write(&run, app_run_script(pkg_name))
        .with_context(|| format!("writing {}", run.display()))?;
    make_executable(&run)?;

    let entry = std::fs::read_to_string(bundle.join(format!("{pkg_name}.desktop"))).ok();
    let entry = match entry {
        Some(text) => appdir_desktop_entry(&text, pkg_name),
        None => default_desktop_entry(pkg_name, app_name),
    };
    let entry_path = app_dir.join(format!("{pkg_name}.desktop"));
    std::fs::write(&entry_path, entry)
        .with_context(|| format!("writing {}", entry_path.display()))?;

    for name in [format!("{pkg_name}.png"), ".DirIcon".to_string()] {
        let dest = app_dir.join(&name);
        std::fs::copy(&icon, &dest).with_context(|| format!("writing {}", dest.display()))?;
    }
    Ok(())
}

/// What the AppImage runs when started: the bundled executable, with whatever
/// arguments it was given.
pub(super) fn app_run_script(pkg_name: &str) -> String {
    format!(
        "#!/bin/sh\n\
         HERE=\"$(dirname \"$(readlink -f \"$0\")\")\"\n\
         exec \"$HERE/usr/bin/{pkg_name}\" \"$@\"\n"
    )
}

/// The bundle's desktop entry, with `Exec` and `Icon` aimed at the AppDir: the
/// launcher by its name, and the icon by the file name the AppDir carries.
pub(super) fn appdir_desktop_entry(contents: &str, pkg_name: &str) -> String {
    let mut out = String::with_capacity(contents.len());
    for line in contents.lines() {
        if line.starts_with("Exec=") {
            out.push_str(&format!("Exec={pkg_name}"));
        } else if line.starts_with("Icon=") {
            out.push_str(&format!("Icon={pkg_name}"));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// A minimal desktop entry, for a bundle that has none.
pub(super) fn default_desktop_entry(pkg_name: &str, app_name: &str) -> String {
    format!(
        "[Desktop Entry]\nName={app_name}\nExec={pkg_name}\nIcon={pkg_name}\n\
         Terminal=false\nType=Application\nCategories=Utility;\n"
    )
}

/// `tar` writing `output` from the folder `top` inside `parent`.
pub(super) fn tar_command(parent: &Path, top: &str, output: &Path) -> Command {
    let mut cmd = Command::new("tar");
    cmd.arg("-czf").arg(output).arg("-C").arg(parent).arg(top);
    cmd
}

/// `appimagetool` turning `app_dir` into `output`.
///
/// `appimagetool` is itself an AppImage, which needs FUSE to start;
/// `APPIMAGE_EXTRACT_AND_RUN` makes it unpack and run instead, so a machine
/// without FUSE (a container, or a recent Ubuntu) works too. `--no-appstream`
/// skips the metadata check that would need `appstreamcli`.
pub(super) fn appimagetool_command(tool: &Path, app_dir: &Path, output: &Path) -> Command {
    let mut cmd = Command::new(tool);
    cmd.arg("--no-appstream")
        .arg(app_dir)
        .arg(output)
        .env("ARCH", appimage_arch())
        .env("APPIMAGE_EXTRACT_AND_RUN", "1");
    cmd
}

/// Copy `src` into `dest`, creating it, directories and all.
fn copy_dir(src: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
    for entry in std::fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            // `copy` carries the permission bits over, so the executable stays
            // executable.
            std::fs::copy(&from, &to)
                .with_context(|| format!("copying {} -> {}", from.display(), to.display()))?;
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests;
