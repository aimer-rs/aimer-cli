use std::path::Path;

use super::install::{
    binary_name, ensure_installed_build, extract_command, find_binary, replace, replace_impl,
};
use super::release::{
    Asset, Decision, Release, archive_name, decide, digest_checksum, parse_checksum, parse_version,
    tag_candidates, target_triple,
};

const HASH: &str = "7573dc04d4bfef321e54f17b061dc9f1ebcb2727ebf5d4b3445f19f55a991404";

fn version(text: &str) -> semver::Version {
    parse_version(text).unwrap()
}

#[test]
fn versions_parse_with_or_without_a_v() {
    assert_eq!(version("0.10.0"), version("v0.10.0"));
    assert!(parse_version("nightly").is_err());
}

#[test]
fn requested_versions_try_both_tag_styles() {
    assert_eq!(tag_candidates("v0.9.1"), ["0.9.1", "v0.9.1"]);
    assert_eq!(tag_candidates("0.9.1"), ["0.9.1", "v0.9.1"]);
}

#[test]
fn every_release_target_has_a_triple() {
    assert_eq!(
        target_triple("macos", "aarch64").unwrap(),
        "aarch64-apple-darwin"
    );
    assert_eq!(
        target_triple("linux", "x86_64").unwrap(),
        "x86_64-unknown-linux-gnu"
    );
    assert_eq!(
        target_triple("linux", "aarch64").unwrap(),
        "aarch64-unknown-linux-gnu"
    );
    assert_eq!(
        target_triple("windows", "x86_64").unwrap(),
        "x86_64-pc-windows-msvc"
    );
    assert_eq!(
        target_triple("windows", "aarch64").unwrap(),
        "aarch64-pc-windows-msvc"
    );
}

#[test]
fn unsupported_platforms_say_so() {
    let err = target_triple("macos", "x86_64").unwrap_err().to_string();
    assert!(err.contains("macos/x86_64"), "{err}");
}

#[test]
fn archive_names_match_the_release_workflow() {
    assert_eq!(
        archive_name("0.10.0", "aarch64-apple-darwin"),
        "aimer-0.10.0-aarch64-apple-darwin.tar.gz"
    );
    assert_eq!(
        archive_name("0.10.0", "x86_64-pc-windows-msvc"),
        "aimer-0.10.0-x86_64-pc-windows-msvc.zip"
    );
}

#[test]
fn checksum_files_yield_the_first_hash() {
    let text = format!("{HASH}  aimer-0.10.0-aarch64-apple-darwin.tar.gz\n");
    assert_eq!(parse_checksum(&text).as_deref(), Some(HASH));
    assert_eq!(parse_checksum(&HASH.to_uppercase()).as_deref(), Some(HASH));
    assert_eq!(parse_checksum("not a hash"), None);
    assert_eq!(parse_checksum(""), None);
}

#[test]
fn github_digests_are_read_as_bare_hex() {
    let asset = |digest: Option<&str>| Asset {
        name: "a".into(),
        browser_download_url: String::new(),
        size: 0,
        digest: digest.map(str::to_string),
    };
    assert_eq!(
        digest_checksum(&asset(Some(&format!("sha256:{HASH}")))).as_deref(),
        Some(HASH)
    );
    assert_eq!(digest_checksum(&asset(Some("sha1:abc"))), None);
    assert_eq!(digest_checksum(&asset(None)), None);
}

#[test]
fn release_json_is_parsed() {
    let json = format!(
        r#"{{"tag_name":"0.10.0","html_url":"https://example/r","extra":1,
            "assets":[{{"name":"a.tar.gz","browser_download_url":"https://example/a",
            "size":12,"digest":"sha256:{HASH}"}}]}}"#
    );
    let release: Release = serde_json::from_str(&json).unwrap();
    assert_eq!(release.version().unwrap(), version("0.10.0"));
    let asset = release.asset("a.tar.gz").unwrap();
    assert_eq!(asset.size, 12);
    assert!(release.asset("missing").is_none());
}

#[test]
fn decisions_follow_the_versions() {
    let (old, new) = (version("0.9.1"), version("0.10.0"));
    assert_eq!(decide(&old, &new, false, false), Decision::Install);
    assert_eq!(decide(&new, &new, false, false), Decision::UpToDate);
    assert_eq!(decide(&new, &old, false, false), Decision::Ahead);
}

#[test]
fn explicit_versions_may_downgrade_and_force_reinstalls() {
    let (old, new) = (version("0.9.1"), version("0.10.0"));
    assert_eq!(decide(&new, &old, true, false), Decision::Install);
    assert_eq!(decide(&new, &new, false, true), Decision::Install);
}

#[test]
fn prereleases_sort_before_their_release() {
    assert_eq!(
        decide(&version("0.11.0-rc.1"), &version("0.11.0"), false, false),
        Decision::Install
    );
}

#[test]
fn archives_are_unpacked_with_tar() {
    let args = |archive: &str| {
        extract_command(Path::new(archive), Path::new("out"))
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(args("a.tar.gz"), ["-xzf", "a.tar.gz", "-C", "out"]);
    assert_eq!(args("a.zip"), ["-xf", "a.zip", "-C", "out"]);
}

#[test]
fn the_binary_is_found_inside_the_release_folder_or_beside_it() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(find_binary(dir.path(), "aimer-1"), None);

    let flat = dir.path().join(binary_name());
    std::fs::write(&flat, "x").unwrap();
    assert_eq!(find_binary(dir.path(), "aimer-1"), Some(flat));

    let folder = dir.path().join("aimer-1");
    std::fs::create_dir(&folder).unwrap();
    let nested = folder.join(binary_name());
    std::fs::write(&nested, "x").unwrap();
    assert_eq!(find_binary(dir.path(), "aimer-1"), Some(nested));
}

#[test]
fn cargo_build_output_is_not_overwritten() {
    assert!(ensure_installed_build(Path::new("/w/target/debug/aimer")).is_err());
    assert!(ensure_installed_build(Path::new("/w/target/release/aimer")).is_err());
    assert!(ensure_installed_build(Path::new("/home/me/.cargo/bin/aimer")).is_ok());
    assert!(ensure_installed_build(Path::new("/usr/local/bin/aimer")).is_ok());
}

#[test]
fn replacing_swaps_the_contents() {
    let dir = tempfile::tempdir().unwrap();
    let current = dir.path().join("aimer");
    let new = dir.path().join("fresh");
    std::fs::write(&current, "old").unwrap();
    std::fs::write(&new, "new").unwrap();

    replace(&current, &new).unwrap();

    assert_eq!(std::fs::read_to_string(&current).unwrap(), "new");
    assert!(!dir.path().join(".aimer.new").exists());
}

#[cfg(unix)]
#[test]
fn the_replacement_is_executable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let current = dir.path().join("aimer");
    let new = dir.path().join("fresh");
    std::fs::write(&current, "old").unwrap();
    std::fs::write(&new, "new").unwrap();

    replace(&current, &new).unwrap();

    let mode = std::fs::metadata(&current).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111);
}

#[test]
fn moving_aside_keeps_the_old_binary() {
    let dir = tempfile::tempdir().unwrap();
    let current = dir.path().join("aimer.exe");
    let new = dir.path().join("fresh");
    std::fs::write(&current, "old").unwrap();
    std::fs::write(&new, "new").unwrap();

    replace_impl(&current, &new, true).unwrap();

    assert_eq!(std::fs::read_to_string(&current).unwrap(), "new");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("aimer.exe.old")).unwrap(),
        "old"
    );
}

#[test]
fn a_failed_replace_leaves_the_binary_alone() {
    let dir = tempfile::tempdir().unwrap();
    let current = dir.path().join("aimer");
    std::fs::write(&current, "old").unwrap();

    let result = replace(&current, &dir.path().join("does-not-exist"));

    assert!(result.is_err());
    assert_eq!(std::fs::read_to_string(&current).unwrap(), "old");
}
