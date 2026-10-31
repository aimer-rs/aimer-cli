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

/// A built app on disk, for the formats that archive it.
fn built_app(root: &Path) -> String {
    let app = root.join("AimerPackageTestApp.app");
    std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    std::fs::write(app.join("Contents/MacOS/AimerPackageTestApp"), b"binary").unwrap();
    std::fs::write(app.join("Contents/Info.plist"), b"<plist/>").unwrap();
    app.to_string_lossy().into_owned()
}

fn archive_job<'a>(
    format: PackageFormat,
    app: &'a str,
    dist: &'a str,
    signing: Option<&'a SigningPlan>,
) -> PackageJob<'a> {
    PackageJob {
        format,
        app_path: app,
        app_name: "AimerPackageTestApp",
        version: "1.0.0",
        dist,
        signing,
        image_tool: ImageTool::Hdiutil,
    }
}

#[test]
fn a_zip_holds_the_app_and_needs_no_tool() {
    let dir = tempfile::tempdir().unwrap();
    let app = built_app(dir.path());
    let dist = dir.path().join("dist");
    let dist = dist.to_str().unwrap();
    let spy = SpyReporter::new();

    let out = wrap_app(&archive_job(PackageFormat::Zip, &app, dist, None), &spy).unwrap();

    assert_eq!(out, format!("{dist}/AimerPackageTestApp-1.0.0.zip"));
    let zip = std::fs::read(&out).unwrap();
    let text = String::from_utf8_lossy(&zip);
    assert!(text.contains("AimerPackageTestApp.app/Contents/MacOS/AimerPackageTestApp"));
    assert!(spy.steps().is_empty());
}

#[test]
fn a_tar_gz_archives_the_app_folder_by_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let app = built_app(dir.path());
    let dist = dir.path().join("dist");
    let dist = dist.to_str().unwrap();
    let spy = SpyReporter::new();

    let out = wrap_app(&archive_job(PackageFormat::TarGz, &app, dist, None), &spy).unwrap();

    assert_eq!(out, format!("{dist}/AimerPackageTestApp-1.0.0.tar.gz"));
    let step = &spy.steps()[0];
    assert_eq!(step.program, "tar");
    assert_eq!(step.args[0], "-czf");
    assert!(step.args[1].ends_with("AimerPackageTestApp-1.0.0.tar.gz"));
    assert_eq!(step.args[2], "-C");
    assert_eq!(Path::new(&step.args[3]), dir.path());
    assert_eq!(step.args[4], "AimerPackageTestApp.app");
}

#[test]
fn a_signed_archive_signs_the_app_before_archiving_it() {
    let dir = tempfile::tempdir().unwrap();
    let app = built_app(dir.path());
    let dist = dir.path().join("dist");
    let dist = dist.to_str().unwrap();
    let spy = SpyReporter::new();
    let signing = plan(None, None);

    wrap_app(
        &archive_job(PackageFormat::TarGz, &app, dist, Some(&signing)),
        &spy,
    )
    .unwrap();

    assert_eq!(spy.programs(), ["codesign", "codesign", "tar"]);
}

#[test]
fn a_notarized_archive_has_the_app_notarized_and_stapled_before_it_goes_in() {
    let dir = tempfile::tempdir().unwrap();
    let app = built_app(dir.path());
    let dist = dir.path().join("dist");
    let dist = dist.to_str().unwrap();
    let spy = SpyReporter::new();
    let signing = plan(Some(profile_auth()), None);

    wrap_app(
        &archive_job(PackageFormat::Zip, &app, dist, Some(&signing)),
        &spy,
    )
    .unwrap();

    let steps = spy.steps();
    let programs: Vec<&str> = steps.iter().map(|s| s.program.as_str()).collect();
    // Sign and verify, submit the zipped app, staple the app. Nothing after:
    // an archive cannot carry a ticket, and the app already has its own.
    assert_eq!(programs, ["codesign", "codesign", "xcrun", "xcrun"]);
    assert_eq!(steps[2].args[..2], ["notarytool", "submit"]);
    assert!(
        steps[2].args[2].ends_with(".notarize-AimerPackageTestApp.zip"),
        "{:?}",
        steps[2].args
    );
    assert_eq!(steps[3].args, ["stapler", "staple", app.as_str()]);
}

#[test]
fn the_notarization_zip_is_removed_and_the_final_archive_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let app = built_app(dir.path());
    let dist = dir.path().join("dist");
    let signing = plan(Some(profile_auth()), None);

    let out = wrap_app(
        &archive_job(
            PackageFormat::Zip,
            &app,
            dist.to_str().unwrap(),
            Some(&signing),
        ),
        &SpyReporter::new(),
    )
    .unwrap();

    assert!(Path::new(&out).is_file());
    assert!(!dist.join(".notarize-AimerPackageTestApp.zip").exists());
}

#[test]
fn a_failed_notarization_leaves_no_archive_and_no_submission_zip() {
    let dir = tempfile::tempdir().unwrap();
    let app = built_app(dir.path());
    let dist = dir.path().join("dist");
    let signing = plan(Some(profile_auth()), None);

    let err = wrap_app(
        &archive_job(
            PackageFormat::TarGz,
            &app,
            dist.to_str().unwrap(),
            Some(&signing),
        ),
        &SpyReporter::failing("notarization"),
    )
    .unwrap_err();

    assert!(err.to_string().contains("notarization failed"), "{err}");
    assert!(!dist.join(".notarize-AimerPackageTestApp.zip").exists());
    assert!(!dist.join("AimerPackageTestApp-1.0.0.tar.gz").exists());
}

#[test]
fn a_dmg_is_still_notarized_as_a_whole() {
    let steps = wrap(PackageFormat::Dmg, Some(&plan(Some(profile_auth()), None)));

    assert_eq!(steps[steps.len() - 2], "xcrun notarytool");
    assert_eq!(steps[steps.len() - 1], "xcrun stapler");
}

#[test]
fn a_format_that_is_not_a_macos_package_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let app = built_app(dir.path());
    let dist = dir.path().join("dist");

    let err = wrap_app(
        &archive_job(PackageFormat::Msi, &app, dist.to_str().unwrap(), None),
        &SpyReporter::new(),
    )
    .unwrap_err();

    assert!(err.to_string().contains("not a macOS package"), "{err}");
}
