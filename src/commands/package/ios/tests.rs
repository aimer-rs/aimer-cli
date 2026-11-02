use std::collections::HashMap;

use super::*;
use crate::commands::package::test_support::{SpyReporter, args_of};

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |key| map.get(key).cloned()
}

fn meta() -> IosSigningMeta {
    IosSigningMeta {
        team_id: Some("TEAMMANIFEST".into()),
        method: None,
        allow_provisioning_updates: None,
    }
}

fn resolve(
    meta: Option<&IosSigningMeta>,
    env: &[(&str, &str)],
    upload: bool,
    release: bool,
) -> anyhow::Result<IosSigningPlan> {
    IosSigningPlan::resolve(meta, &env_of(env), upload, release)
}

fn plan() -> IosSigningPlan {
    IosSigningPlan {
        team_id: "TEAM123456".into(),
        method: APP_STORE_METHOD.into(),
        allow_provisioning_updates: false,
        key: None,
    }
}

fn key() -> AscKey {
    AscKey {
        path: "AuthKey.p8".into(),
        key_id: "KID".into(),
        issuer: "ISS".into(),
    }
}

#[test]
fn the_app_store_is_the_default_method_and_updates_are_off() {
    let plan = resolve(Some(&meta()), &[], false, true).unwrap();

    assert_eq!(plan.team_id, "TEAMMANIFEST");
    assert_eq!(plan.method, APP_STORE_METHOD);
    assert!(!plan.allow_provisioning_updates);
    assert_eq!(plan.key, None);
}

#[test]
fn the_environment_overrides_the_manifest() {
    let env = [
        (ENV_TEAM_ID, "TEAMENV"),
        (ENV_METHOD, "release-testing"),
        (ENV_PROVISIONING_UPDATES, "true"),
    ];
    let plan = resolve(Some(&meta()), &env, false, true).unwrap();

    assert_eq!(plan.team_id, "TEAMENV");
    assert_eq!(plan.method, "release-testing");
    assert!(plan.allow_provisioning_updates);
}

#[test]
fn provisioning_updates_can_be_enabled_in_the_manifest() {
    let allowed = IosSigningMeta {
        allow_provisioning_updates: Some(true),
        ..meta()
    };

    assert!(
        resolve(Some(&allowed), &[], false, true)
            .unwrap()
            .allow_provisioning_updates
    );
    let env = [(ENV_PROVISIONING_UPDATES, "0")];
    assert!(
        !resolve(Some(&allowed), &env, false, true)
            .unwrap()
            .allow_provisioning_updates
    );
}

#[test]
fn a_missing_team_names_both_places_to_set_it() {
    let err = resolve(None, &[], false, true).unwrap_err().to_string();

    assert!(err.contains("[ios.signing]"), "{err}");
    assert!(err.contains(ENV_TEAM_ID), "{err}");
}

#[test]
fn an_unknown_method_lists_the_valid_ones() {
    let err = resolve(Some(&meta()), &[(ENV_METHOD, "ad-hoc")], false, true)
        .unwrap_err()
        .to_string();

    assert!(err.contains("ad-hoc"), "{err}");
    assert!(err.contains("release-testing"), "{err}");
}

#[test]
fn an_upload_needs_the_app_store_method_and_a_release_build() {
    let testing = [(ENV_METHOD, "release-testing")];
    let err = resolve(Some(&meta()), &testing, true, true).unwrap_err();
    assert!(err.to_string().contains(APP_STORE_METHOD), "{err}");

    let err = resolve(Some(&meta()), &[], true, false).unwrap_err();
    assert!(err.to_string().contains("release"), "{err}");

    assert!(resolve(Some(&meta()), &[], true, true).is_ok());
}

#[test]
fn the_api_key_is_all_or_nothing() {
    let all = [
        (ENV_ASC_KEY, "AuthKey.p8"),
        (ENV_ASC_KEY_ID, "KID"),
        (ENV_ASC_ISSUER, "ISS"),
    ];
    assert_eq!(
        resolve(Some(&meta()), &all, false, true).unwrap().key,
        Some(key())
    );

    let err = resolve(Some(&meta()), &[(ENV_ASC_KEY, "AuthKey.p8")], false, true)
        .unwrap_err()
        .to_string();
    assert!(err.contains(ENV_ASC_KEY_ID), "{err}");
    assert!(err.contains(ENV_ASC_ISSUER), "{err}");
}

#[test]
fn a_missing_key_file_fails_the_preflight() {
    let with_key = IosSigningPlan {
        key: Some(AscKey {
            path: "/nonexistent/AuthKey.p8".into(),
            ..key()
        }),
        ..plan()
    };

    let err = with_key.preflight().unwrap_err().to_string();
    assert!(err.contains("/nonexistent/AuthKey.p8"), "{err}");
    assert!(plan().preflight().is_ok());
}

fn job<'a>(dist: &'a str, upload: bool) -> ArchiveJob<'a> {
    ArchiveJob {
        pkg_name: "my_app",
        app_name: "My App",
        version: "1.2.3",
        release: true,
        dist,
        upload,
    }
}

#[test]
fn archive_builds_the_scheme_for_any_device_and_signs_with_the_team() {
    let cmd = archive_command(
        &job("dist", false),
        &plan(),
        Path::new("/w/My App.xcarchive"),
    );

    assert_eq!(cmd.get_program(), "xcodebuild");
    assert_eq!(cmd.get_current_dir(), Some(Path::new(IOS_PROJECT_DIR)));
    assert_eq!(
        args_of(&cmd),
        [
            "archive",
            "-project",
            "my_app.xcodeproj",
            "-scheme",
            "my_app",
            "-configuration",
            "Release",
            "-destination",
            "generic/platform=iOS",
            "-archivePath",
            "/w/My App.xcarchive",
            "PRODUCT_NAME=My App",
            "DEVELOPMENT_TEAM=TEAM123456",
            "CODE_SIGN_STYLE=Automatic",
        ]
    );
}

#[test]
fn provisioning_updates_and_the_key_reach_both_xcodebuild_calls() {
    let plan = IosSigningPlan {
        allow_provisioning_updates: true,
        key: Some(key()),
        ..plan()
    };
    let expected = [
        "-allowProvisioningUpdates",
        "-authenticationKeyPath",
        "AuthKey.p8",
        "-authenticationKeyID",
        "KID",
        "-authenticationKeyIssuerID",
        "ISS",
    ];

    let archive = args_of(&archive_command(
        &job("dist", false),
        &plan,
        Path::new("/a"),
    ));
    let export = args_of(&export_command(
        Path::new("/a"),
        Path::new("/e"),
        Path::new("/p.plist"),
        &plan,
    ));
    assert!(
        archive.ends_with(&expected.map(String::from)),
        "{archive:?}"
    );
    assert!(export.ends_with(&expected.map(String::from)), "{export:?}");
}

#[test]
fn nothing_account_changing_is_passed_by_default() {
    let archive = args_of(&archive_command(
        &job("dist", false),
        &plan(),
        Path::new("/a"),
    ));

    assert!(!archive.contains(&"-allowProvisioningUpdates".to_string()));
}

#[test]
fn export_names_the_archive_the_destination_and_the_options() {
    let cmd = export_command(
        Path::new("/w/A.xcarchive"),
        Path::new("/w/export"),
        Path::new("/w/ExportOptions.plist"),
        &plan(),
    );

    assert_eq!(
        args_of(&cmd),
        [
            "-exportArchive",
            "-archivePath",
            "/w/A.xcarchive",
            "-exportPath",
            "/w/export",
            "-exportOptionsPlist",
            "/w/ExportOptions.plist",
        ]
    );
}

#[test]
fn the_export_options_carry_method_team_and_destination() {
    let local = export_options_plist(&plan(), false);

    assert!(
        local.contains("<key>method</key>\n\t<string>app-store-connect</string>"),
        "{local}"
    );
    assert!(
        local.contains("<key>teamID</key>\n\t<string>TEAM123456</string>"),
        "{local}"
    );
    assert!(
        local.contains("<key>signingStyle</key>\n\t<string>automatic</string>"),
        "{local}"
    );
    assert!(
        local.contains("<key>destination</key>\n\t<string>export</string>"),
        "{local}"
    );
    assert!(
        local.contains("<key>uploadSymbols</key>\n\t<true/>"),
        "{local}"
    );

    let upload = export_options_plist(&plan(), true);
    assert!(
        upload.contains("<key>destination</key>\n\t<string>upload</string>"),
        "{upload}"
    );
}

#[test]
fn symbols_are_only_uploaded_for_the_app_store() {
    let testing = IosSigningPlan {
        method: "release-testing".into(),
        ..plan()
    };

    assert!(!export_options_plist(&testing, false).contains("uploadSymbols"));
}

#[test]
fn plist_values_are_escaped() {
    let odd = IosSigningPlan {
        team_id: "A&B<C>".into(),
        ..plan()
    };

    assert!(export_options_plist(&odd, false).contains("A&amp;B&lt;C&gt;"));
}

#[test]
fn the_version_is_set_through_plistbuddy() {
    let cmd = plist_set_command(
        Path::new("/a/Info.plist"),
        "CFBundleShortVersionString",
        "1.2.3",
    );

    assert_eq!(cmd.get_program(), "/usr/libexec/PlistBuddy");
    assert_eq!(
        args_of(&cmd),
        [
            "-c",
            "Set :CFBundleShortVersionString 1.2.3",
            "/a/Info.plist"
        ]
    );
}

/// A spy that writes the `.ipa` a real export would leave in its `-exportPath`.
fn exporting_spy() -> SpyReporter {
    SpyReporter::with_hook(|cmd| {
        let args = args_of(cmd);
        if let Some(at) = args.iter().position(|a| a == "-exportPath") {
            let dir = Path::new(&args[at + 1]);
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("My App.ipa"), b"ipa").unwrap();
        }
    })
}

fn export_to(dist: &Path, upload: bool, spy: &SpyReporter) -> anyhow::Result<String> {
    let dist = dist.to_str().unwrap();
    export_ipa(&job(dist, upload), &plan(), spy)
}

#[test]
fn an_ipa_is_archived_patched_exported_and_named_after_the_version() {
    let dist = tempfile::tempdir().unwrap();
    let spy = exporting_spy();

    let ipa = export_to(dist.path(), false, &spy).unwrap();

    assert_eq!(ipa, format!("{}/My App-1.2.3.ipa", dist.path().display()));
    assert_eq!(std::fs::read(&ipa).unwrap(), b"ipa");
    let actions: Vec<String> = spy.steps().into_iter().map(|s| s.action).collect();
    assert_eq!(
        actions,
        [
            "xcodebuild archive for iOS",
            "PlistBuddy",
            "xcodebuild export"
        ]
    );
    assert!(spy.notes().iter().any(|n| n.starts_with("Archiving")));
    assert!(!dist.path().join(WORK_DIR_NAME).exists());
}

#[test]
fn the_assets_go_into_the_app_inside_the_archive() {
    let dist = tempfile::tempdir().unwrap();
    let spy = exporting_spy();

    export_to(dist.path(), false, &spy).unwrap();

    // The test reporter's `stage_assets` is the trait default, which notes the
    // destination it was given.
    let note = spy
        .notes()
        .into_iter()
        .find(|n| n.starts_with("Copying assets into"))
        .unwrap()
        .replace('\\', "/");
    assert!(
        note.ends_with("My App.xcarchive/Products/Applications/My App.app"),
        "{note}"
    );
}

#[test]
fn an_upload_is_a_second_export_to_apple() {
    let dist = tempfile::tempdir().unwrap();
    let spy = exporting_spy();

    export_to(dist.path(), true, &spy).unwrap();

    let steps = spy.steps();
    assert_eq!(steps.len(), 4);
    assert_eq!(steps[3].action, "xcodebuild export (upload)");
    assert!(steps[3].args.contains(&"-exportArchive".to_string()));
}

#[test]
fn a_failed_archive_stops_before_any_export_and_cleans_up() {
    let dist = tempfile::tempdir().unwrap();
    let spy = SpyReporter::failing("xcodebuild archive for iOS");

    let err = export_to(dist.path(), true, &spy).unwrap_err();

    assert!(err.to_string().contains("archive"), "{err}");
    assert_eq!(spy.steps().len(), 1);
    assert!(!dist.path().join(WORK_DIR_NAME).exists());
}

#[test]
fn an_export_without_an_ipa_is_an_error() {
    let dist = tempfile::tempdir().unwrap();
    // A plain spy never writes the file a real export would.
    let spy = SpyReporter::new();

    // The export directory does not even exist, which is how a skipped or
    // silently failing export looks.
    let err = export_to(dist.path(), false, &spy).unwrap_err();

    assert!(err.to_string().contains("export"), "{err}");
}

#[test]
fn a_failed_upload_still_leaves_the_exported_ipa() {
    let dist = tempfile::tempdir().unwrap();
    let spy = exporting_spy().failing_on("xcodebuild export (upload)");

    let err = export_to(dist.path(), true, &spy).unwrap_err();

    assert!(err.to_string().contains("upload"), "{err}");
    assert!(dist.path().join("My App-1.2.3.ipa").is_file());
    assert!(!dist.path().join(WORK_DIR_NAME).exists());
}
