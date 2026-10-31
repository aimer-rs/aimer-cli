use std::collections::HashMap;

use super::*;

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |key| map.get(key).cloned()
}

fn meta() -> MacosSigningMeta {
    MacosSigningMeta {
        identity: Some("Developer ID Application: Manifest".into()),
        installer_identity: Some("Developer ID Installer: Manifest".into()),
        entitlements: Some("app.entitlements".into()),
        notary_profile: Some("manifest-profile".into()),
    }
}

fn resolve(
    meta: Option<&MacosSigningMeta>,
    env: &[(&str, &str)],
    format: PackageFormat,
    notarize: bool,
) -> anyhow::Result<SigningPlan> {
    SigningPlan::resolve(meta, &env_of(env), format, notarize)
}

#[test]
fn manifest_values_are_used_when_the_environment_is_empty() {
    let plan = resolve(Some(&meta()), &[], PackageFormat::Dmg, true).unwrap();

    assert_eq!(plan.identity, "Developer ID Application: Manifest");
    assert_eq!(plan.entitlements.as_deref(), Some("app.entitlements"));
    assert_eq!(
        plan.notary,
        Some(NotaryAuth::KeychainProfile("manifest-profile".into()))
    );
}

#[test]
fn the_environment_overrides_the_manifest() {
    let env = [
        (ENV_IDENTITY, "Developer ID Application: Env"),
        (ENV_NOTARY_PROFILE, "env-profile"),
    ];
    let plan = resolve(Some(&meta()), &env, PackageFormat::Dmg, true).unwrap();

    assert_eq!(plan.identity, "Developer ID Application: Env");
    assert_eq!(
        plan.notary,
        Some(NotaryAuth::KeychainProfile("env-profile".into()))
    );
}

#[test]
fn an_empty_variable_counts_as_unset() {
    let plan = resolve(
        Some(&meta()),
        &[(ENV_IDENTITY, "  ")],
        PackageFormat::Dmg,
        false,
    )
    .unwrap();

    assert_eq!(plan.identity, "Developer ID Application: Manifest");
}

#[test]
fn signing_without_an_identity_names_both_places_to_set_it() {
    let err = resolve(None, &[], PackageFormat::Dmg, false)
        .unwrap_err()
        .to_string();

    assert!(err.contains("[macos.signing]"), "{err}");
    assert!(err.contains(ENV_IDENTITY), "{err}");
}

#[test]
fn a_pkg_requires_an_installer_identity_but_a_dmg_does_not() {
    let app_only = MacosSigningMeta {
        identity: Some("Developer ID Application: Me".into()),
        ..MacosSigningMeta::default()
    };

    let err = resolve(Some(&app_only), &[], PackageFormat::Pkg, false).unwrap_err();
    assert!(err.to_string().contains("Installer"), "{err}");

    let dmg = resolve(Some(&meta()), &[], PackageFormat::Dmg, false).unwrap();
    assert_eq!(dmg.installer_identity, None);
}

#[test]
fn notary_credentials_are_only_required_when_notarizing() {
    let app_only = MacosSigningMeta {
        identity: Some("Developer ID Application: Me".into()),
        ..MacosSigningMeta::default()
    };

    assert!(resolve(Some(&app_only), &[], PackageFormat::Dmg, false).is_ok());
    let err = resolve(Some(&app_only), &[], PackageFormat::Dmg, true).unwrap_err();
    assert!(err.to_string().contains("notary credentials"), "{err}");
}

#[test]
fn a_complete_api_key_wins_over_a_profile() {
    let env = [
        (ENV_NOTARY_KEY, "AuthKey.p8"),
        (ENV_NOTARY_KEY_ID, "ABC123"),
        (ENV_NOTARY_ISSUER, "issuer-uuid"),
    ];
    let plan = resolve(Some(&meta()), &env, PackageFormat::Dmg, true).unwrap();

    assert_eq!(
        plan.notary,
        Some(NotaryAuth::ApiKey {
            path: "AuthKey.p8".into(),
            key_id: "ABC123".into(),
            issuer: "issuer-uuid".into(),
        })
    );
}

#[test]
fn a_partial_api_key_names_the_missing_variables() {
    let err = resolve(
        Some(&meta()),
        &[(ENV_NOTARY_KEY, "AuthKey.p8")],
        PackageFormat::Dmg,
        true,
    )
    .unwrap_err()
    .to_string();

    assert!(err.contains(ENV_NOTARY_KEY_ID), "{err}");
    assert!(err.contains(ENV_NOTARY_ISSUER), "{err}");
    assert!(!err.contains("missing: AIMER_NOTARY_KEY,"), "{err}");
}

#[test]
fn identities_are_matched_against_the_keychain_listing() {
    let listing = r#"  1) 0123ABCD "Developer ID Application: Me (TEAM1234)"
  2) 4567EF01 "Developer ID Installer: Me (TEAM1234)"
     2 valid identities found"#;

    assert!(identity_listed(listing, "Developer ID Application: Me"));
    assert!(identity_listed(listing, "4567EF01"));
    assert!(!identity_listed(
        listing,
        "Developer ID Application: Someone Else"
    ));
}

#[test]
fn the_app_is_signed_with_the_hardened_runtime_and_entitlements() {
    let plan = SigningPlan {
        identity: "Developer ID Application: Me".into(),
        installer_identity: None,
        entitlements: Some("app.entitlements".into()),
        notary: None,
    };
    let cmd = codesign_app_command(&plan, "App.app");

    assert_eq!(cmd.get_program(), "codesign");
    assert_eq!(
        args_of(&cmd),
        [
            "--force",
            "--options",
            "runtime",
            "--timestamp",
            "--entitlements",
            "app.entitlements",
            "--sign",
            "Developer ID Application: Me",
            "App.app",
        ]
    );
}

#[test]
fn entitlements_are_left_out_when_not_configured() {
    let plan = SigningPlan {
        identity: "ID".into(),
        installer_identity: None,
        entitlements: None,
        notary: None,
    };

    assert_eq!(
        args_of(&codesign_app_command(&plan, "App.app")),
        [
            "--force",
            "--options",
            "runtime",
            "--timestamp",
            "--sign",
            "ID",
            "App.app"
        ]
    );
}

#[test]
fn a_disk_image_is_signed_without_the_runtime_option() {
    assert_eq!(
        args_of(&codesign_image_command("ID", "App.dmg")),
        ["--force", "--timestamp", "--sign", "ID", "App.dmg"]
    );
    assert_eq!(
        args_of(&codesign_verify_command("App.app")),
        ["--verify", "--strict", "--verbose=2", "App.app"]
    );
}

#[test]
fn notarization_waits_for_the_verdict_with_the_chosen_credentials() {
    let profile = notarize_command(&NotaryAuth::KeychainProfile("p".into()), "App.dmg");
    assert_eq!(profile.get_program(), "xcrun");
    assert_eq!(
        args_of(&profile),
        [
            "notarytool",
            "submit",
            "App.dmg",
            "--wait",
            "--keychain-profile",
            "p"
        ]
    );

    let key = notarize_command(
        &NotaryAuth::ApiKey {
            path: "k.p8".into(),
            key_id: "KID".into(),
            issuer: "ISS".into(),
        },
        "App.pkg",
    );
    assert_eq!(
        args_of(&key),
        [
            "notarytool",
            "submit",
            "App.pkg",
            "--wait",
            "--key",
            "k.p8",
            "--key-id",
            "KID",
            "--issuer",
            "ISS",
        ]
    );
}

#[test]
fn stapling_targets_the_artifact() {
    assert_eq!(
        args_of(&staple_command("App.dmg")),
        ["stapler", "staple", "App.dmg"]
    );
}

fn args_of(cmd: &Command) -> Vec<String> {
    cmd.get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect()
}
