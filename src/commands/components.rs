//! `aimer components`: manage the tools Aimer installs for itself.
//!
//! Components live in Aimer's own directory, `~/.aimer`, apart from the user's
//! `~/.cargo/bin`. Cargo tools are built with `cargo install --root ~/.aimer`
//! into `~/.aimer/bin`, so cargo keeps the install records and
//! `cargo uninstall --root` can take them out again. The Android NDK is
//! downloaded from Google into `~/.aimer/ndk` (see [`ndk`]). The installer
//! generators `aimer package` needs (`appimagetool`, Inno Setup, WiX) are
//! managed too (see [`packagers`]).
//! [`expose_on_path`] appends the `bin` directory to `PATH`, so a tool is found
//! whenever the user has not got it on `PATH` already.

mod catalog;
mod completion;
mod ndk;
mod packagers;

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use colored::Colorize;

use catalog::{COMPONENTS, Component, Source};

pub(crate) use completion::{available as complete_available, installed as complete_installed};
pub(crate) use ndk::ensure_available as ensure_ndk;

#[cfg(test)]
mod tests;

/// The directory holding Aimer's data, `~/.aimer`.
pub(crate) fn aimer_root() -> anyhow::Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("cannot determine the user's home directory")?
        .join(".aimer"))
}

/// Where the components of the install `root` keep their executables.
pub(crate) fn bin_dir(root: &Path) -> PathBuf {
    root.join("bin")
}

fn binary_path(root: &Path, binary: &str) -> PathBuf {
    let file = if cfg!(windows) {
        format!("{binary}.exe")
    } else {
        binary.to_string()
    };
    bin_dir(root).join(file)
}

fn is_installed(root: &Path, component: &Component) -> bool {
    match component.source {
        Source::Cargo { binary, .. } | Source::Dotnet { binary, .. } => {
            binary_path(root, binary).is_file()
        }
        Source::AppImageTool => binary_path(root, "appimagetool").is_file(),
        Source::InnoSetup => packagers::iscc_path(root).is_file(),
        Source::Ndk => !ndk::installed(root).is_empty(),
    }
}

/// `PATH` with the managed `bin` directory added at the end, unless it is
/// already there. It goes last so a tool the user installed themselves wins.
pub(crate) fn path_with_managed_bin(path: Option<OsString>, bin: &Path) -> OsString {
    let mut entries: Vec<PathBuf> = path
        .as_deref()
        .map(|path| env::split_paths(path).collect())
        .unwrap_or_default();
    if !entries.iter().any(|entry| entry == bin) {
        entries.push(bin.to_path_buf());
    }
    env::join_paths(entries).unwrap_or_default()
}

/// Make the executables in `~/.aimer/bin` visible to the commands Aimer runs.
///
/// Does nothing when the directory does not exist, so users who never run
/// `aimer components add` see no difference.
pub fn expose_on_path() {
    let Ok(root) = aimer_root() else { return };
    let bin = bin_dir(&root);
    if !bin.is_dir() {
        return;
    }
    let path = path_with_managed_bin(env::var_os("PATH"), &bin);
    // SAFETY: called at the start of `main`, before any thread is spawned.
    unsafe { env::set_var("PATH", path) };
}

/// A component named on the command line, with the version asked for in
/// `name@version` form.
#[derive(Debug, PartialEq, Eq)]
struct Selection {
    component: &'static Component,
    version: Option<String>,
}

/// Look up each of `names`, failing on the first unknown one. Only the NDK
/// accepts a version (`ndk@27`).
fn resolve(names: &[String]) -> anyhow::Result<Vec<Selection>> {
    let mut resolved: Vec<Selection> = Vec::new();
    for name in names {
        let (name, version) = match name.split_once('@') {
            Some((name, version)) => (name, Some(version.to_string())),
            None => (name.as_str(), None),
        };
        let component = catalog::find(name).with_context(|| {
            format!(
                "unknown component '{name}' (available: {})",
                catalog::names()
            )
        })?;
        if version.is_some() && component.source != Source::Ndk {
            bail!("component '{name}' does not take a version");
        }
        let selection = Selection { component, version };
        if !resolved.contains(&selection) {
            resolved.push(selection);
        }
    }
    Ok(resolved)
}

/// The command that installs the cargo `package` into the install `root`.
fn install_command(root: &Path, package: &str) -> Command {
    let mut cmd = cargo();
    cmd.arg("install")
        .arg("--locked")
        .arg("--root")
        .arg(root)
        .arg(package);
    cmd
}

/// The command that removes the cargo `package` from the install `root`.
fn uninstall_command(root: &Path, package: &str) -> Command {
    let mut cmd = cargo();
    cmd.arg("uninstall").arg("--root").arg(root).arg(package);
    cmd
}

/// The command that installs the .NET tool `package` into the `bin` directory
/// of the install `root`.
fn dotnet_install_command(root: &Path, package: &str) -> Command {
    let mut cmd = Command::new("dotnet");
    cmd.args(["tool", "install", "--tool-path"])
        .arg(bin_dir(root))
        .arg(package);
    cmd
}

/// The command that removes the .NET tool `package` from the install `root`.
fn dotnet_uninstall_command(root: &Path, package: &str) -> Command {
    let mut cmd = Command::new("dotnet");
    cmd.args(["tool", "uninstall", "--tool-path"])
        .arg(bin_dir(root))
        .arg(package);
    cmd
}

fn cargo() -> Command {
    Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn run(mut cmd: Command) -> bool {
    cmd.status().map(|status| status.success()).unwrap_or(false)
}

/// Print every component and whether it is installed.
fn list(root: &Path) {
    println!("{}", "Aimer components".bold());
    println!("{}", format!("  installed in {}", root.display()).dimmed());
    for component in COMPONENTS {
        // Pad before colouring so ANSI codes don't break alignment.
        let name = format!("{:<14}", component.name);
        let (mark, name) = if is_installed(root, component) {
            ("✔".green(), name.green())
        } else {
            ("✘".red(), name.red())
        };
        let detail = match component.source {
            Source::Ndk => {
                let versions = ndk::installed(root);
                if versions.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", versions.join(", "))
                }
            }
            _ if !component.supported_here() => {
                format!(" ({} only)", component.only_on.unwrap_or_default())
            }
            _ => String::new(),
        };
        println!(
            "  {mark}  {name} {}",
            format!("— {}{detail}", component.purpose).dimmed()
        );
    }
}

/// Install one selection.
fn install_one(root: &Path, selection: &Selection, accept_licenses: bool) -> anyhow::Result<()> {
    let component = selection.component;
    if !component.supported_here() {
        bail!(
            "{} can only be installed on {}",
            component.name,
            component.only_on.unwrap_or_default()
        );
    }
    match component.source {
        Source::Cargo { package, .. } => {
            if run(install_command(root, package)) {
                Ok(())
            } else {
                bail!("`cargo install {package}` failed")
            }
        }
        Source::Dotnet { package, .. } => {
            if is_installed(root, component) {
                println!("{} is already installed", component.name);
                Ok(())
            } else if run(dotnet_install_command(root, package)) {
                Ok(())
            } else {
                bail!("`dotnet tool install {package}` failed (is the .NET SDK installed?)")
            }
        }
        Source::AppImageTool => packagers::install_appimagetool(root),
        Source::InnoSetup => packagers::install_inno_setup(root),
        Source::Ndk => ndk::install(root, selection.version.as_deref(), accept_licenses),
    }
}

/// Install `names` into the install `root`. `accept_licenses` accepts any
/// licence a component requires without asking.
fn add(root: &Path, names: &[String], accept_licenses: bool) -> anyhow::Result<()> {
    let selections = resolve(names)?;
    let mut failed = Vec::new();
    for selection in selections {
        let name = selection.component.name;
        println!("{} {name}", "Installing".bold());
        match install_one(root, &selection, accept_licenses) {
            Ok(()) => println!("{} {name}", "✔".green()),
            Err(error) => {
                eprintln!("{error:#}");
                println!("{} {name}", "✘".red());
                failed.push(name);
            }
        }
    }
    if !failed.is_empty() {
        bail!("failed to install: {}", failed.join(", "));
    }
    Ok(())
}

/// Remove the executable `binary` from the managed `bin` directory, first
/// letting `uninstall` (the tool's own package manager) try. `Ok(false)` means
/// it was not installed.
fn remove_binary(
    root: &Path,
    binary: &str,
    uninstall: impl FnOnce() -> bool,
) -> anyhow::Result<bool> {
    let path = binary_path(root, binary);
    if !path.is_file() {
        return Ok(false);
    }
    // A binary dropped into the directory by hand has no package-manager
    // record, so the uninstall refuses it; delete the file then.
    if uninstall() || std::fs::remove_file(&path).is_ok() {
        Ok(true)
    } else {
        bail!("could not remove {}", path.display())
    }
}

/// Remove one selection. `Ok(false)` means it was not installed.
fn remove_one(root: &Path, selection: &Selection) -> anyhow::Result<bool> {
    match selection.component.source {
        Source::Cargo { package, binary } => {
            remove_binary(root, binary, || run(uninstall_command(root, package)))
        }
        Source::Dotnet { package, binary } => remove_binary(root, binary, || {
            run(dotnet_uninstall_command(root, package))
        }),
        Source::AppImageTool => remove_binary(root, "appimagetool", || false),
        Source::InnoSetup => packagers::remove_inno_setup(root),
        Source::Ndk => Ok(!ndk::remove(root, selection.version.as_deref())?.is_empty()),
    }
}

/// Remove `names` from the install `root`.
fn remove(root: &Path, names: &[String]) -> anyhow::Result<()> {
    let selections = resolve(names)?;
    let mut failed = Vec::new();
    for selection in selections {
        let name = selection.component.name;
        println!("{} {name}", "Removing".bold());
        match remove_one(root, &selection) {
            Ok(true) => println!("{} {name}", "✔".green()),
            Ok(false) => println!("{} {name} is not installed", "•".dimmed()),
            Err(error) => {
                eprintln!("{error:#}");
                println!("{} {name}", "✘".red());
                failed.push(name);
            }
        }
    }
    if !failed.is_empty() {
        bail!("failed to remove: {}", failed.join(", "));
    }
    Ok(())
}

/// What `aimer components` was asked to do.
pub enum Action {
    List,
    Add {
        names: Vec<String>,
        accept_licenses: bool,
    },
    Remove(Vec<String>),
}

pub fn execute(action: Action) -> anyhow::Result<()> {
    let root = aimer_root()?;
    match action {
        Action::List => {
            list(&root);
            Ok(())
        }
        Action::Add {
            names,
            accept_licenses,
        } => add(&root, &names, accept_licenses),
        Action::Remove(names) => remove(&root, &names),
    }
}
