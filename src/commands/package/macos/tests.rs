use super::*;
use crate::commands::package::test_support::{SpyReporter, args_of};

#[test]
fn pkgbuild_installs_the_component_into_applications() {
    let cmd = pkgbuild_command("build/Release/My App.app", "dist/My App.pkg", None);

    assert_eq!(cmd.get_program(), "pkgbuild");
    assert_eq!(
        args_of(&cmd),
        [
            "--component",
            "build/Release/My App.app",
            "--install-location",
            "/Applications",
            "dist/My App.pkg",
        ]
    );
}

#[test]
fn pkgbuild_signs_with_the_installer_identity() {
    let cmd = pkgbuild_command("App.app", "App.pkg", Some("Developer ID Installer: Me"));

    assert_eq!(
        args_of(&cmd),
        [
            "--component",
            "App.app",
            "--install-location",
            "/Applications",
            "--sign",
            "Developer ID Installer: Me",
            "--timestamp",
            "App.pkg",
        ]
    );
}

#[test]
fn stale_package_is_removed_but_a_missing_one_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    let stale = dir.path().join("App.dmg");
    std::fs::write(&stale, b"old").unwrap();

    remove_stale(stale.to_str().unwrap()).unwrap();
    assert!(!stale.exists());
    remove_stale(stale.to_str().unwrap()).unwrap();
}

fn plan(notary: Option<NotaryAuth>, installer: Option<&str>) -> SigningPlan {
    SigningPlan {
        identity: "Developer ID Application: Me".into(),
        installer_identity: installer.map(String::from),
        entitlements: None,
        notary,
    }
}

fn profile_auth() -> NotaryAuth {
    NotaryAuth::KeychainProfile("notary".into())
}

/// Run `wrap_app` in a scratch directory with the `hdiutil` image tool (whose
/// steps do not depend on Finder) and return every step as
/// `<program> <first argument>`, in order.
fn wrap(format: PackageFormat, signing: Option<&SigningPlan>) -> Vec<String> {
    let dist = tempfile::tempdir().unwrap();
    let reporter = SpyReporter::new();
    let job = PackageJob {
        format,
        app_path: "App.app",
        app_name: "AimerPackageTestApp",
        version: "1.0.0",
        dist: dist.path().to_str().unwrap(),
        signing,
        image_tool: ImageTool::Hdiutil,
    };
    wrap_app(&job, &reporter).unwrap();

    reporter
        .steps()
        .iter()
        .map(|s| {
            format!(
                "{} {}",
                s.program,
                s.args.first().cloned().unwrap_or_default()
            )
        })
        .collect()
}

fn programs(steps: &[String]) -> Vec<&str> {
    steps.iter().map(|s| s.split(' ').next().unwrap()).collect()
}

#[test]
fn an_unsigned_dmg_runs_no_signing_steps() {
    let steps = wrap(PackageFormat::Dmg, None);

    assert_eq!(programs(&steps), ["ditto", "ln", "hdiutil"]);
}

#[test]
fn a_signed_dmg_signs_the_app_then_the_image() {
    let steps = wrap(PackageFormat::Dmg, Some(&plan(None, None)));

    assert_eq!(
        programs(&steps),
        ["codesign", "codesign", "ditto", "ln", "hdiutil", "codesign"]
    );
}

#[test]
fn a_notarized_dmg_ends_with_submit_and_staple() {
    let steps = wrap(PackageFormat::Dmg, Some(&plan(Some(profile_auth()), None)));

    assert_eq!(steps[steps.len() - 2], "xcrun notarytool");
    assert_eq!(steps[steps.len() - 1], "xcrun stapler");
}

#[test]
fn a_signed_pkg_signs_the_app_and_lets_pkgbuild_sign_the_package() {
    let steps = wrap(
        PackageFormat::Pkg,
        Some(&plan(
            Some(profile_auth()),
            Some("Developer ID Installer: Me"),
        )),
    );

    // App signature + verification, pkgbuild (signs itself), notarize, staple.
    assert_eq!(
        programs(&steps),
        ["codesign", "codesign", "pkgbuild", "xcrun", "xcrun"]
    );
}
