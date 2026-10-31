use super::*;

fn options(platform: &str) -> PackageOptions {
    PackageOptions {
        platform: platform.to_string(),
        format: None,
        release: true,
        package: None,
        sign: false,
        notarize: false,
        upload: false,
    }
}

#[test]
fn macos_defaults_to_dmg() {
    let request = resolve_request(&options("macos")).unwrap();

    assert_eq!(request.target, Targets::Macos);
    assert_eq!(request.format, PackageFormat::Dmg);
}

#[test]
fn ios_defaults_to_ipa() {
    let request = resolve_request(&options("ios")).unwrap();

    assert_eq!(request.target, Targets::Ios);
    assert_eq!(request.format, PackageFormat::Ipa);
}

#[test]
fn an_explicit_format_overrides_the_default() {
    let request = resolve_request(&PackageOptions {
        format: Some(PackageFormat::Pkg),
        ..options("macos")
    })
    .unwrap();

    assert_eq!(request.format, PackageFormat::Pkg);
}

#[test]
fn a_format_must_belong_to_the_platform() {
    for (platform, format) in [
        ("macos", PackageFormat::Ipa),
        ("ios", PackageFormat::Dmg),
        ("ios", PackageFormat::Pkg),
    ] {
        let err = resolve_request(&PackageOptions {
            format: Some(format),
            ..options(platform)
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("not available"),
            "{platform}: {err}"
        );
    }
}

#[test]
fn platforms_other_than_macos_and_ios_are_rejected() {
    for platform in ["windows", "linux", "android", "web"] {
        let err = resolve_request(&options(platform)).unwrap_err();
        assert!(
            err.to_string().contains("not supported yet"),
            "{platform}: {err}"
        );
    }
}

#[test]
fn an_unknown_platform_is_reported_as_such() {
    assert!(resolve_request(&options("amiga")).is_err());
}

#[test]
fn flags_aimed_at_the_other_platform_are_rejected() {
    let notarize = resolve_request(&PackageOptions {
        notarize: true,
        ..options("ios")
    })
    .unwrap_err();
    assert!(notarize.to_string().contains("--notarize"), "{notarize}");

    let upload = resolve_request(&PackageOptions {
        upload: true,
        ..options("macos")
    })
    .unwrap_err();
    assert!(upload.to_string().contains("--upload"), "{upload}");

    assert!(
        resolve_request(&PackageOptions {
            upload: true,
            ..options("ios")
        })
        .is_ok()
    );
}

#[test]
fn package_stem_appends_a_version_only_when_there_is_one() {
    assert_eq!(package_stem("My App", "1.2.3"), "My App-1.2.3");
    assert_eq!(package_stem("My App", ""), "My App");
    assert_eq!(package_stem("My App", "  "), "My App");
}

#[test]
fn packages_land_in_a_per_platform_and_profile_dist_directory() {
    assert_eq!(dist_dir(Targets::Macos, false), "builds/macos/dist/debug");
    assert_eq!(dist_dir(Targets::Macos, true), "builds/macos/dist/release");
    assert_eq!(dist_dir(Targets::Ios, true), "builds/ios/dist/release");
}

#[test]
fn formats_map_to_their_file_extension() {
    assert_eq!(PackageFormat::Dmg.extension(), "dmg");
    assert_eq!(PackageFormat::Pkg.extension(), "pkg");
    assert_eq!(PackageFormat::Ipa.extension(), "ipa");
}
