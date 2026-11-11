use std::process::Command;
use std::sync::Mutex;

use super::*;
use crate::commands::assemble::Step;

struct SpyReporter(Mutex<Vec<String>>);

impl SpyReporter {
    fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    fn notes(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl Reporter for SpyReporter {
    fn note(&self, message: String) {
        self.0.lock().unwrap().push(message);
    }

    fn run(&self, _cmd: Command, _step: Step) -> anyhow::Result<()> {
        Ok(())
    }
}

const PLIST: &str = "<plist version=\"1.0\">\n<dict>\n    <key>NSCameraUsageDescription</key>\n    <string>Mine</string>\n</dict>\n</plist>\n";

fn entry(key: &str, value: &str) -> (String, String) {
    (key.to_string(), value.to_string())
}

#[test]
fn an_override_is_reported_and_the_old_entry_survives_as_a_comment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Info.plist");
    std::fs::write(&path, PLIST).unwrap();
    let reporter = SpyReporter::new();

    sync_plist_file(
        &path,
        &[entry("NSCameraUsageDescription", "Theirs")],
        &reporter,
    )
    .unwrap();

    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("<string>Mine</string> -->"), "{written}");
    assert!(written.contains("<string>Theirs</string>"), "{written}");
    let notes = reporter.notes();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(
        notes[0].contains("NSCameraUsageDescription was \"Mine\", Aimer.toml sets \"Theirs\""),
        "{notes:?}"
    );
    assert!(
        notes[1].contains("wrote NSCameraUsageDescription"),
        "{notes:?}"
    );
}

#[test]
fn a_synced_file_is_not_rewritten_by_the_next_sync() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Info.plist");
    std::fs::write(&path, PLIST).unwrap();
    let wanted = [entry("NSMicrophoneUsageDescription", "Record")];
    sync_plist_file(&path, &wanted, &SpyReporter::new()).unwrap();
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));

    sync_plist_file(&path, &wanted, &SpyReporter::new()).unwrap();

    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before
    );
}

#[test]
fn a_missing_file_is_a_warning_only_when_permissions_are_declared() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Info.plist");
    let quiet = SpyReporter::new();
    let loud = SpyReporter::new();

    sync_plist_file(&path, &[], &quiet).unwrap();
    sync_plist_file(&path, &[entry("A", "b")], &loud).unwrap();

    assert!(quiet.notes().is_empty());
    assert!(
        loud.notes()[0].starts_with("warning:"),
        "{:?}",
        loud.notes()
    );
    assert!(!path.exists());
}

#[test]
fn removing_every_permission_removes_the_block_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("AndroidManifest.xml");
    let manifest = "<manifest>\n\n    <application/>\n</manifest>\n";
    std::fs::write(&path, manifest).unwrap();
    let uses = [AndroidUse {
        name: "android.permission.CAMERA".into(),
        max_sdk_version: None,
    }];
    sync_android_file(&path, &uses, &SpyReporter::new()).unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("CAMERA"));
    let reporter = SpyReporter::new();

    sync_android_file(&path, &[], &reporter).unwrap();

    assert_eq!(std::fs::read_to_string(&path).unwrap(), manifest);
    assert!(
        reporter.notes()[0].contains("removed the generated block"),
        "{:?}",
        reporter.notes()
    );
}
