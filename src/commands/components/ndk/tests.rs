use std::io::Cursor;
use std::path::Path;

use super::license::{self, License};
use super::manifest::{self, Manifest};
use super::{download, installed};

const XML: &str = r#"<sdk:sdk-repository>
  <license id="android-sdk-license" type="text">Terms &amp; Conditions
&quot;quoted&quot;</license>
  <license id="android-sdk-preview-license" type="text">Preview terms</license>
  <remotePackage path="ndk-bundle"><revision><major>17</major></revision></remotePackage>
  <remotePackage path="ndk;29.0.14206865">
    <revision><major>29</major><minor>0</minor><micro>14206865</micro></revision>
    <uses-license ref="android-sdk-license"/>
    <archives>
      <archive><complete><size>10</size><checksum type="sha1">AA11</checksum><url>android-ndk-r29-linux.zip</url></complete><host-os>linux</host-os></archive>
      <archive><complete><size>20</size><checksum type="sha1">bb22</checksum><url>android-ndk-r29-darwin.zip</url></complete><host-os>macosx</host-os></archive>
    </archives>
  </remotePackage>
  <remotePackage path="ndk;30.0.1">
    <revision><major>30</major><minor>0</minor><micro>1</micro><preview>1</preview></revision>
    <uses-license ref="android-sdk-preview-license"/>
    <archives>
      <archive><complete><size>1</size><checksum type="sha1">cc33</checksum><url>beta-darwin.zip</url></complete><host-os>macosx</host-os></archive>
    </archives>
  </remotePackage>
  <remotePackage path="ndk;27.3.13750724">
    <revision><major>27</major><minor>3</minor><micro>13750724</micro></revision>
    <uses-license ref="android-sdk-license"/>
    <archives>
      <archive><complete><size>30</size><checksum type="sha1">dd44</checksum><url>android-ndk-r27d-darwin.zip</url></complete><host-os>macosx</host-os></archive>
    </archives>
  </remotePackage>
  <remotePackage path="ndk;21.4.7075529">
    <revision><major>21</major><minor>4</minor><micro>7075529</micro></revision>
    <uses-license ref="android-sdk-license"/>
    <archives>
      <archive><complete><size>40</size><checksum type="sha1">ee55</checksum><url>old-darwin.zip</url></complete><host-os>macosx</host-os></archive>
    </archives>
  </remotePackage>
</sdk:sdk-repository>"#;

fn mac() -> Manifest {
    Manifest::parse(XML, "macosx")
}

fn terms() -> License<'static> {
    License {
        id: "android-sdk-license",
        text: "the terms",
    }
}

fn ask(
    root: &Path,
    typed: &str,
    assume_yes: bool,
    interactive: bool,
) -> (anyhow::Result<()>, String) {
    let mut out = Vec::new();
    let result = license::ensure_accepted(
        root,
        &terms(),
        assume_yes,
        interactive,
        &mut Cursor::new(typed.to_string()),
        &mut out,
    );
    (result, String::from_utf8(out).unwrap())
}

#[test]
fn picks_the_newest_stable_release_for_the_host() {
    let manifest = mac();
    let release = manifest.select(None).unwrap();
    assert_eq!(release.version, "29.0.14206865");
    assert_eq!(release.license, "android-sdk-license");
    assert_eq!(release.archive.size, 20);
    assert_eq!(release.archive.sha1, "bb22");
    assert_eq!(
        release.archive.url,
        "https://dl.google.com/android/repository/android-ndk-r29-darwin.zip"
    );
}

#[test]
fn skips_previews_and_ndks_older_than_r23() {
    let manifest = mac();
    let error = manifest.select(Some("30")).unwrap_err().to_string();
    assert!(error.contains("29.0.14206865"));
    assert!(error.contains("27.3.13750724"));
    assert!(!error.contains("21.4"));
}

#[test]
fn only_releases_with_a_download_for_the_host_count() {
    let manifest = Manifest::parse(XML, "linux");
    assert_eq!(manifest.select(None).unwrap().version, "29.0.14206865");
    assert!(manifest.select(Some("27")).is_err());
}

#[test]
fn a_version_can_be_a_prefix_of_the_full_one() {
    let manifest = mac();
    for wanted in ["27", "r27", "27.3", "27.3.13750724"] {
        assert_eq!(
            manifest.select(Some(wanted)).unwrap().version,
            "27.3.13750724",
            "{wanted}"
        );
    }
    assert!(manifest.select(Some("2")).is_err());
    assert!(manifest.select(Some("")).is_err());
}

#[test]
fn version_matching_does_not_match_partial_numbers() {
    assert!(!manifest::version_matches("27.3.1", "27.3.1x"));
    assert!(!manifest::version_matches("127.0.1", "27"));
    assert!(manifest::version_matches("27.3.1", "R27"));
}

#[test]
fn licence_text_is_unescaped() {
    let manifest = mac();
    assert_eq!(
        manifest.license_text("android-sdk-license"),
        Some("Terms & Conditions\n\"quoted\"")
    );
    assert_eq!(manifest.license_text("missing"), None);
}

#[test]
fn an_empty_manifest_has_nothing_to_install() {
    let manifest = Manifest::parse("<nothing/>", "macosx");
    assert!(manifest.select(None).is_err());
}

#[test]
fn pressing_enter_accepts_and_the_answer_is_remembered() {
    let root = tempfile::tempdir().unwrap();
    let (result, shown) = ask(root.path(), "\n", false, true);
    result.unwrap();
    assert!(shown.contains("the terms"));
    assert!(shown.contains("Press Enter"));
    assert!(license::is_accepted(root.path(), &terms()));

    // Remembered: no prompt and no input needed the second time.
    let (result, shown) = ask(root.path(), "", false, true);
    result.unwrap();
    assert!(shown.is_empty());
}

#[test]
fn yes_also_accepts() {
    let root = tempfile::tempdir().unwrap();
    ask(root.path(), "yes\n", false, true).0.unwrap();
    assert!(license::is_accepted(root.path(), &terms()));
}

#[test]
fn declining_does_not_record_anything() {
    let root = tempfile::tempdir().unwrap();
    let (result, _) = ask(root.path(), "n\n", false, true);
    assert!(result.unwrap_err().to_string().contains("not accepted"));
    assert!(!license::is_accepted(root.path(), &terms()));
}

#[test]
fn closed_input_declines() {
    let root = tempfile::tempdir().unwrap();
    assert!(ask(root.path(), "", false, true).0.is_err());
}

#[test]
fn a_changed_licence_text_asks_again() {
    let root = tempfile::tempdir().unwrap();
    ask(root.path(), "\n", false, true).0.unwrap();
    let changed = License {
        id: "android-sdk-license",
        text: "new terms",
    };
    assert!(!license::is_accepted(root.path(), &changed));
}

#[test]
fn without_a_terminal_the_licence_needs_the_flag() {
    let root = tempfile::tempdir().unwrap();
    let (result, _) = ask(root.path(), "\n", false, false);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("--accept-licenses")
    );
    assert!(!license::is_accepted(root.path(), &terms()));

    ask(root.path(), "", true, false).0.unwrap();
    assert!(license::is_accepted(root.path(), &terms()));
}

#[test]
fn licence_ids_cannot_escape_the_licences_directory() {
    let root = tempfile::tempdir().unwrap();
    let evil = License {
        id: "../escape",
        text: "x",
    };
    let result = license::ensure_accepted(
        root.path(),
        &evil,
        true,
        false,
        &mut Cursor::new(String::new()),
        &mut Vec::new(),
    );
    assert!(result.is_err());
}

#[test]
fn confirm_treats_enter_as_yes() {
    let mut out = Vec::new();
    assert!(license::confirm(&mut Cursor::new("\n"), &mut out, "ok? ").unwrap());
    assert!(!license::confirm(&mut Cursor::new("no\n"), &mut out, "ok? ").unwrap());
    assert!(!license::confirm(&mut Cursor::new(""), &mut out, "ok? ").unwrap());
}

#[test]
fn progress_shows_percent_and_megabytes() {
    let mb = 1024 * 1024;
    assert_eq!(
        download::progress_line(330 * mb, 783 * mb),
        "   42%  330/783 MB"
    );
    assert_eq!(download::progress_line(0, 0), "    0%  0/0 MB");
    assert_eq!(download::progress_line(5 * mb, 4 * mb), "  100%  5/4 MB");
}

#[test]
fn extraction_unpacks_into_the_destination() {
    let cmd = download::extract_command(Path::new("/tmp/ndk.zip"), Path::new("/tmp/out"));
    let args: Vec<String> = cmd
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if cfg!(windows) {
        assert_eq!(cmd.get_program(), "tar");
        assert_eq!(args, ["-xf", "/tmp/ndk.zip", "-C", "/tmp/out"]);
    } else {
        assert_eq!(cmd.get_program(), "unzip");
        assert_eq!(args, ["-q", "/tmp/ndk.zip", "-d", "/tmp/out"]);
    }
}

#[test]
fn the_unpacked_ndk_is_the_directory_with_source_properties() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(download::unpacked_ndk(dir.path()), None);
    std::fs::create_dir(dir.path().join("other")).unwrap();
    let ndk = dir.path().join("android-ndk-r27d");
    std::fs::create_dir(&ndk).unwrap();
    std::fs::write(ndk.join("source.properties"), "Pkg.Revision = 27.3.1\n").unwrap();
    assert_eq!(download::unpacked_ndk(dir.path()), Some(ndk));
}

#[test]
fn installed_lists_versions_oldest_first_and_ignores_staging() {
    let root = tempfile::tempdir().unwrap();
    assert!(installed(root.path()).is_empty());
    let base = crate::commands::aimer_ndk::managed_dir(root.path());
    for version in ["27.10.1", "27.9.1", ".download-x"] {
        let dir = base.join(version);
        std::fs::create_dir_all(&dir).unwrap();
        if version != ".download-x" {
            std::fs::write(dir.join("source.properties"), "").unwrap();
        }
    }
    assert_eq!(installed(root.path()), ["27.9.1", "27.10.1"]);
}
