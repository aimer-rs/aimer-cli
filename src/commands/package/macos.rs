//! Wrapping a macOS `.app` bundle into a `.dmg` or `.pkg`.
//!
//! The `.dmg` itself is built by [`super::dmg`]. Every external tool is built as
//! a plain [`Command`] and handed to the [`Reporter`], so the argument lists can
//! be asserted without touching the machine.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};

use super::dmg::{self, DmgRequest, ImageTool};
use super::sign::{self, NotaryAuth, SigningPlan};
use super::{PackageFormat, Request, dist_dir, package_stem, remove_stale};
use crate::commands::assemble::{self, Reporter, StdioReporter, Step, StepKind, profile_name};
use crate::config::{AimerManifest, resolve_cargo_package_name};

/// Build the macOS app and wrap it as the request says; returns the package.
pub(super) fn run(request: &Request, manifest: Option<&AimerManifest>) -> anyhow::Result<String> {
    let (release, format) = (request.release, request.format);

    // Resolved and checked before the (slow) build, so a missing certificate or
    // a mistyped notary profile fails in seconds.
    let signing = if request.sign || request.notarize {
        let meta = manifest
            .and_then(|m| m.macos.as_ref())
            .and_then(|m| m.signing.as_ref());
        let plan = SigningPlan::resolve(
            meta,
            &|key| std::env::var(key).ok(),
            format,
            request.notarize,
        )?;
        sign::preflight(&plan)?;
        Some(plan)
    } else {
        None
    };

    println!(
        "Packaging '{}' app into a {}.{} in {} mode...",
        request.target,
        if signing.is_some() { "signed " } else { "" },
        format.extension(),
        profile_name(release)
    );

    let reporter = StdioReporter;
    let pkg_name = resolve_cargo_package_name(Path::new("."));
    assemble::build_macos(release, &reporter)?;
    let app_path = assemble::package_macos(&pkg_name, release, &reporter)?;

    let version = manifest
        .map(|manifest| manifest.package.version.clone())
        .unwrap_or_default();
    let app_name = assemble::app_name();
    let dist = dist_dir(request.target, release);
    let job = PackageJob {
        format,
        app_path: &app_path,
        app_name: &app_name,
        version: &version,
        dist: &dist,
        signing: signing.as_ref(),
        image_tool: ImageTool::detect(),
    };
    wrap_app(&job, &reporter)
}

/// One package to produce: the built `.app`, how to wrap it, and whether to
/// sign and notarize it.
pub(super) struct PackageJob<'a> {
    pub format: PackageFormat,
    pub app_path: &'a str,
    pub app_name: &'a str,
    pub version: &'a str,
    /// Directory the package is written to.
    pub dist: &'a str,
    pub signing: Option<&'a SigningPlan>,
    /// The tool family that creates the `.dmg`.
    pub image_tool: ImageTool,
}

/// Wrap the app of `job` into its format and return the path of the package,
/// relative to the project root.
///
/// With a signing plan the app is signed first (after its assets have been
/// staged, so the signature covers them), then the package is signed, and with
/// notary credentials it is finally notarized and stapled.
pub(super) fn wrap_app(job: &PackageJob, reporter: &dyn Reporter) -> anyhow::Result<String> {
    let dist = job.dist;
    std::fs::create_dir_all(dist).with_context(|| format!("creating {dist}"))?;
    let output = format!(
        "{dist}/{}.{}",
        package_stem(job.app_name, job.version),
        job.format.extension()
    );
    remove_stale(&output)?;

    if let Some(plan) = job.signing {
        sign_app(plan, job.app_path, reporter)?;
    }

    match job.format {
        PackageFormat::Dmg => {
            let request = DmgRequest {
                app_path: job.app_path,
                app_name: job.app_name,
                dist,
                output: &output,
                tool: job.image_tool,
                volume_mounted: dmg::volume_is_mounted(job.app_name),
            };
            dmg::build_dmg(&request, reporter)?;
            if let Some(plan) = job.signing {
                reporter.note(format!("Signing {output}..."));
                reporter.run(
                    sign::codesign_image_command(&plan.identity, &output),
                    Step::new(StepKind::Other, "codesign for the disk image"),
                )?;
            }
        }
        PackageFormat::Pkg => {
            reporter.note(format!("Building installer package {output}..."));
            let installer = job.signing.and_then(|p| p.installer_identity.as_deref());
            reporter.run(
                pkgbuild_command(job.app_path, &output, installer),
                Step::new(StepKind::Other, "pkgbuild"),
            )?;
        }
        other => bail!("a .{} is not a macOS package", other.extension()),
    }

    if let Some(auth) = job.signing.and_then(|plan| plan.notary.as_ref()) {
        notarize(auth, &output, reporter)?;
    }
    Ok(output)
}

fn sign_app(plan: &SigningPlan, app_path: &str, reporter: &dyn Reporter) -> anyhow::Result<()> {
    reporter.note(format!("Signing {app_path}..."));
    reporter.run(
        sign::codesign_app_command(plan, app_path),
        Step::new(StepKind::Other, "codesign for the app"),
    )?;
    reporter.run(
        sign::codesign_verify_command(app_path),
        Step::new(StepKind::Other, "codesign verification"),
    )
}

fn notarize(auth: &NotaryAuth, artifact: &str, reporter: &dyn Reporter) -> anyhow::Result<()> {
    reporter.note(format!(
        "Submitting {artifact} to the Apple notary service (this can take several minutes)..."
    ));
    reporter.run(
        sign::notarize_command(auth, artifact),
        Step::new(StepKind::Other, "notarization"),
    )?;
    reporter.note(format!("Stapling the notarization ticket to {artifact}..."));
    reporter.run(
        sign::staple_command(artifact),
        Step::new(StepKind::Other, "stapler"),
    )
}

/// Build a flat component package installing the app into `/Applications`,
/// signed with `installer_identity` when one is given.
///
/// The package identifier and version are left to `pkgbuild`, which reads them
/// from the bundle's `Info.plist`.
pub(super) fn pkgbuild_command(
    app_path: &str,
    output: &str,
    installer_identity: Option<&str>,
) -> Command {
    let mut cmd = Command::new("pkgbuild");
    cmd.args(["--component", app_path])
        .args(["--install-location", "/Applications"]);
    if let Some(identity) = installer_identity {
        cmd.args(["--sign", identity, "--timestamp"]);
    }
    cmd.arg(output);
    cmd
}

#[cfg(test)]
mod tests;
