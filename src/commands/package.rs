mod android;
mod dmg;
mod ios;
mod linux;
mod macos;
mod sign;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

use std::path::Path;

use anyhow::{Context, bail};
use clap::ValueEnum;

use crate::commands::assemble::profile_name;
use crate::config::AimerManifest;
use crate::errors::AimerError;
use crate::targets::Targets;

/// The installable container `aimer package` wraps the platform bundle in.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum PackageFormat {
    /// A disk image with the app next to an `Applications` shortcut.
    Dmg,
    /// A flat installer package that installs the app into `/Applications`.
    Pkg,
    /// An iOS app archive, ready for App Store Connect or ad-hoc installs.
    Ipa,
    /// An Android App Bundle, the format Google Play takes.
    Aab,
    /// An Android package that installs directly, outside Google Play.
    Apk,
    /// A single-file Linux app that runs on most distributions.
    #[value(name = "appimage")]
    Appimage,
    /// The portable Linux bundle, compressed.
    #[value(name = "tar.gz", alias = "tar-gz")]
    TarGz,
}

impl PackageFormat {
    /// The file extension of the produced package, without the dot.
    pub(crate) fn extension(self) -> &'static str {
        match self {
            PackageFormat::Dmg => "dmg",
            PackageFormat::Pkg => "pkg",
            PackageFormat::Ipa => "ipa",
            PackageFormat::Aab => "aab",
            PackageFormat::Apk => "apk",
            PackageFormat::Appimage => "AppImage",
            PackageFormat::TarGz => "tar.gz",
        }
    }

    /// The format used when `--format` is not given for `target`.
    ///
    /// `None` means packaging is not available for that platform yet.
    pub(crate) fn default_for(target: Targets) -> Option<Self> {
        match target {
            Targets::Macos => Some(PackageFormat::Dmg),
            Targets::Ios => Some(PackageFormat::Ipa),
            Targets::Android => Some(PackageFormat::Aab),
            Targets::Linux => Some(PackageFormat::Appimage),
            _ => None,
        }
    }

    /// Whether `target` can be packaged into this format.
    pub(crate) fn supports(self, target: Targets) -> bool {
        match self {
            PackageFormat::Dmg | PackageFormat::Pkg => target == Targets::Macos,
            PackageFormat::Ipa => target == Targets::Ios,
            PackageFormat::Aab | PackageFormat::Apk => target == Targets::Android,
            PackageFormat::Appimage | PackageFormat::TarGz => target == Targets::Linux,
        }
    }
}

/// Where the packages of a build mode are written, relative to the project root.
pub(crate) fn dist_dir(target: Targets, release: bool) -> String {
    let platform = match target {
        Targets::Ios => "ios",
        Targets::Android => "android",
        Targets::Linux => "linux",
        _ => "macos",
    };
    format!("builds/{platform}/dist/{}", profile_name(release))
}

/// File name (without extension) of a package: `<app>-<version>`, or just the
/// app name when `Aimer.toml` declares no version.
pub(crate) fn package_stem(app_name: &str, version: &str) -> String {
    let version = version.trim();
    if version.is_empty() {
        app_name.to_string()
    } else {
        format!("{app_name}-{version}")
    }
}

/// Empty `dir`, creating it when absent.
fn reset_dir(dir: &str) -> anyhow::Result<()> {
    if Path::new(dir).exists() {
        std::fs::remove_dir_all(dir).with_context(|| format!("clearing {dir}"))?;
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {dir}"))
}

/// Remove a package left over from an earlier run, so a failed build can never
/// leave an outdated package behind looking current.
fn remove_stale(output: &str) -> anyhow::Result<()> {
    match std::fs::remove_file(output) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing stale package {output}")),
    }
}

/// Everything `aimer package` was asked to do.
#[derive(Clone, Debug)]
pub struct PackageOptions {
    /// The platform to package, as typed (`macos`, `ios`, ...).
    pub platform: String,
    /// `--format`; each platform has a default.
    pub format: Option<PackageFormat>,
    /// Build in release mode.
    pub release: bool,
    /// `--package`: the Aimer package of a Cargo workspace.
    pub package: Option<String>,
    /// macOS: sign the app and the package.
    pub sign: bool,
    /// macOS: sign, then have Apple notarize the package.
    pub notarize: bool,
    /// iOS: upload the build to App Store Connect.
    pub upload: bool,
}

/// A validated [`PackageOptions`]: the platform and format are known to belong
/// together, and no flag is aimed at the wrong platform.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Request {
    pub target: Targets,
    pub format: PackageFormat,
    pub release: bool,
    pub sign: bool,
    pub notarize: bool,
    pub upload: bool,
}

/// Resolve the platform and format of an `aimer package` invocation.
///
/// Runs before project selection so an unsupported combination is reported on
/// its own terms rather than masked by an unrelated "no Aimer project found".
pub(crate) fn resolve_request(options: &PackageOptions) -> anyhow::Result<Request> {
    let platform = options.platform.as_str();
    let target =
        Targets::try_from(platform).map_err(|_| AimerError::UnknownTarget(platform.to_string()))?;
    let Some(default) = PackageFormat::default_for(target) else {
        bail!(
            "packaging for '{target}' is not supported yet; only 'macos', 'ios', 'android' and 'linux' can be packaged"
        );
    };
    let format = options.format.unwrap_or(default);
    if !format.supports(target) {
        bail!(
            "the '{}' format is not available for '{target}'",
            format.extension()
        );
    }
    if target == Targets::Ios && options.notarize {
        bail!("--notarize applies to macOS packages; iOS builds go through App Store Connect");
    }
    if target == Targets::Android && options.notarize {
        bail!("--notarize applies to macOS packages only");
    }
    if target == Targets::Linux && options.notarize {
        bail!("--notarize applies to macOS packages only");
    }
    if target == Targets::Linux && options.sign {
        bail!("code signing is not available for Linux packages yet");
    }
    if target == Targets::Android && !options.release {
        bail!(
            "Android packages are signed release builds; \
             use `aimer assemble android` for a debug build"
        );
    }
    if target != Targets::Ios && options.upload {
        bail!("--upload is only available for iOS");
    }
    Ok(Request {
        target,
        format,
        release: options.release,
        sign: options.sign,
        notarize: options.notarize,
        upload: options.upload,
    })
}

/// `aimer package <platform>`: build the app, then wrap it in a distributable
/// package: a `.dmg` (the default) or `.pkg` on macOS, an `.ipa` on iOS, and an
/// `.aab` (the default) or `.apk` on Android, and an `.AppImage` (the default) or
/// `.tar.gz` on Linux.
///
/// On macOS, `sign` signs the app and the package with the identities from
/// `[macos.signing]`, and `notarize` implies `sign` and also has Apple notarize
/// the package and staples the ticket to it. On iOS the app is always signed
/// (`[ios.signing]`), and `upload` sends the build to App Store Connect. On
/// Android the package is always a signed release build (`[android.signing]`).
pub fn execute_with_package(options: PackageOptions) -> anyhow::Result<()> {
    let request = resolve_request(&options)?;
    // Apple's packages are made with Xcode tooling and Linux's with the host's
    // own tools (a desktop build is never cross-compiled); Android's only need
    // Gradle and a JDK, so they work on any host.
    let host_ok = match request.target {
        Targets::Android => true,
        Targets::Linux => cfg!(target_os = "linux"),
        _ => cfg!(target_os = "macos"),
    };
    if !host_ok {
        bail!(
            "packaging a '{}' app into a .{} needs a {} host",
            request.target,
            request.format.extension(),
            if request.target == Targets::Linux {
                "Linux"
            } else {
                "macOS"
            }
        );
    }

    let project =
        crate::commands::project_selection::select_aimer_project(options.package.as_deref())?;
    let _project_directory = project.enter()?;
    let manifest = AimerManifest::load_from(Path::new("."))?;

    let artifact = match request.target {
        Targets::Ios => ios::run(&request, manifest.as_ref())?,
        Targets::Android => android::run(&request, manifest.as_ref())?,
        Targets::Linux => linux::run(&request, manifest.as_ref())?,
        _ => macos::run(&request, manifest.as_ref())?,
    };

    println!("Package created successfully: {artifact}");
    Ok(())
}
