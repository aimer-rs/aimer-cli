use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use super::*;
use crate::commands::assemble::Step;

struct Notes(Mutex<Vec<String>>);

impl Notes {
    fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl Reporter for Notes {
    fn note(&self, message: String) {
        self.0.lock().unwrap().push(message);
    }

    fn run(&self, _cmd: Command, _step: Step) -> anyhow::Result<()> {
        Ok(())
    }
}

/// A scratch project with the iOS scaffold in it; returns `builds/ios`.
fn scaffolded(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "aimer-live-activity-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    crate::commands::create::ios::create(&dir, name, name, "com.example.app");
    dir.join("builds/ios")
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).unwrap()
}

#[test]
fn the_feature_of_aimer_turns_it_on() {
    assert!(detect::enabled(
        "[dependencies]\naimer = { path = \"..\", features = [\"markdown\", \"live-activity\"] }\n"
    ));
}

#[test]
fn the_crate_itself_turns_it_on() {
    assert!(detect::enabled("[dependencies]\naimer-live-activity = \"0.1\"\n"));
    assert!(detect::enabled("[dependencies]\naimer_live_activity.workspace = true\n"));
}

#[test]
fn a_target_specific_dependency_counts() {
    assert!(detect::enabled(
        "[target.'cfg(target_os = \"ios\")'.dependencies]\naimer = { version = \"1\", features = [\"live-activity\"] }\n"
    ));
}

#[test]
fn other_features_and_broken_manifests_do_not() {
    assert!(!detect::enabled("[dependencies]\naimer = { features = [\"markdown\"] }\n"));
    assert!(!detect::enabled("[dependencies]\naimer = \"1\"\n"));
    assert!(!detect::enabled("[dependencies]\nother = { features = [\"live-activity\"] }\n"));
    assert!(!detect::enabled("not [ toml"));
}

#[test]
fn the_bundle_identifier_must_be_spelled_out() {
    assert_eq!(
        bundle_identifier("<key>CFBundleIdentifier</key>\n<string>com.example.app</string>").as_deref(),
        Some("com.example.app")
    );
    assert_eq!(
        bundle_identifier("<key>CFBundleIdentifier</key><string>$(PRODUCT_BUNDLE_IDENTIFIER)</string>"),
        None
    );
}

#[test]
fn a_module_name_is_a_swift_identifier() {
    assert_eq!(identifier("my-app LiveActivity"), "my_app_LiveActivity");
    assert_eq!(identifier("3dLiveActivity"), "_3dLiveActivity");
}

#[test]
fn the_sync_generates_the_files_the_key_and_the_target() {
    let ios = scaffolded("my_app");
    let notes = Notes::new();

    sync_project(&ios, "my_app", &notes).unwrap();

    assert!(read(ios.join("my_app/AimerLiveActivityBridge.swift")).contains("@_cdecl"));
    assert!(read(ios.join("my_app/AimerLiveActivityAttributes.swift")).contains("ActivityAttributes"));
    let widget = read(ios.join("my_appLiveActivity/AimerLiveActivityWidget.swift"));
    assert!(widget.contains("ActivityConfiguration"), "{widget}");
    let extension_plist = read(ios.join("my_appLiveActivity/Info.plist"));
    assert!(extension_plist.contains("com.apple.widgetkit-extension"), "{extension_plist}");
    assert!(!extension_plist.contains("${"), "{extension_plist}");

    let app_plist = read(ios.join("my_app/Info.plist"));
    assert!(app_plist.contains("<key>NSSupportsLiveActivities</key>\n    <true/>"), "{app_plist}");
    assert!(app_plist.trim_end().ends_with("</plist>"), "{app_plist}");

    let project = read(ios.join("my_app.xcodeproj/project.pbxproj"));
    for expected in [
        "isa = PBXNativeTarget;\n\t\t\tbuildConfigurationList = AE1A",
        "productType = \"com.apple.product-type.app-extension\"",
        "PRODUCT_BUNDLE_IDENTIFIER = \"com.example.app.LiveActivity\"",
        "PRODUCT_MODULE_NAME = my_appLiveActivity",
        "dstSubfolderSpec = 13",
        "isa = PBXTargetDependency",
        "AimerLiveActivityBridge.swift in Sources",
    ] {
        assert!(project.contains(expected), "missing {expected}");
    }

    let said = notes.all().join("\n");
    assert!(said.contains("com.example.app.LiveActivity"), "{said}");
    assert!(said.contains("never overwritten"), "{said}");
    assert!(!said.contains("warning"), "{said}");
}

#[test]
fn the_patched_project_stays_well_formed() {
    let ios = scaffolded("my_app");
    sync_project(&ios, "my_app", &Notes::new()).unwrap();
    let project = read(ios.join("my_app.xcodeproj/project.pbxproj"));

    let (open, close) = (project.matches('{').count(), project.matches('}').count());
    assert_eq!(open, close, "unbalanced braces");
    let (open, close) = (project.matches('(').count(), project.matches(')').count());
    assert_eq!(open, close, "unbalanced parentheses");

    // Every identifier the extension introduces is defined and used.
    for n in 1..=20u32 {
        if matches!(n, 15) {
            continue;
        }
        let id = format!("AE1A{n:020X}");
        assert!(project.matches(&id).count() >= 2, "{id} is not defined and referenced");
    }
}

#[test]
fn a_second_run_changes_nothing_and_says_nothing() {
    let ios = scaffolded("my_app");
    sync_project(&ios, "my_app", &Notes::new()).unwrap();
    let project = read(ios.join("my_app.xcodeproj/project.pbxproj"));
    let plist = read(ios.join("my_app/Info.plist"));
    let notes = Notes::new();

    sync_project(&ios, "my_app", &notes).unwrap();

    assert_eq!(read(ios.join("my_app.xcodeproj/project.pbxproj")), project);
    assert_eq!(read(ios.join("my_app/Info.plist")), plist);
    assert!(notes.all().is_empty(), "{:?}", notes.all());
}

#[test]
fn an_edited_widget_is_never_overwritten() {
    let ios = scaffolded("my_app");
    sync_project(&ios, "my_app", &Notes::new()).unwrap();
    let widget = ios.join("my_appLiveActivity/AimerLiveActivityWidget.swift");
    fs::write(&widget, "// mine\n").unwrap();

    sync_project(&ios, "my_app", &Notes::new()).unwrap();

    assert_eq!(read(&widget), "// mine\n");
}

#[test]
fn a_project_that_left_the_scaffold_is_not_touched_and_the_user_is_told() {
    let ios = scaffolded("my_app");
    let path = ios.join("my_app.xcodeproj/project.pbxproj");
    let edited = read(&path).replace("/* End PBXSourcesBuildPhase section */", "");
    fs::write(&path, &edited).unwrap();
    let notes = Notes::new();

    sync_project(&ios, "my_app", &notes).unwrap();

    assert_eq!(read(&path), edited, "the project must stay as it was");
    let said = notes.all().join("\n");
    assert!(said.contains("warning: Live activity: the Xcode project was not changed"), "{said}");
    assert!(said.contains("End PBXSourcesBuildPhase"), "{said}");
    // The files that do not depend on the project are still written.
    assert!(ios.join("my_appLiveActivity/AimerLiveActivityWidget.swift").exists());
}

#[test]
fn a_missing_ios_project_is_a_warning_not_an_error() {
    let dir = std::env::temp_dir().join(format!("aimer-live-activity-none-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let notes = Notes::new();

    sync_project(&dir, "my_app", &notes).unwrap();

    assert!(notes.all()[0].contains("not found"), "{:?}", notes.all());
}

#[test]
fn an_app_whose_plist_uses_the_build_setting_gets_the_identifier_from_the_project() {
    let ios = scaffolded("my_app");
    let plist_path = ios.join("my_app/Info.plist");
    let plist = read(&plist_path).replace("<string>com.example.app</string>", "<string>$(PRODUCT_BUNDLE_IDENTIFIER)</string>");
    fs::write(&plist_path, plist).unwrap();
    let project_path = ios.join("my_app.xcodeproj/project.pbxproj");
    let project = read(&project_path).replace(
        "\t\t\t\tSDKROOT = iphoneos;\n\t\t\t\tTARGETED_DEVICE_FAMILY = \"1,2\";\n\t\t\t};\n\t\t\tname = Debug;",
        "\t\t\t\tPRODUCT_BUNDLE_IDENTIFIER = dev.example.unquoted;\n\t\t\t\tSDKROOT = iphoneos;\n\t\t\t\tTARGETED_DEVICE_FAMILY = \"1,2\";\n\t\t\t};\n\t\t\tname = Debug;",
    );
    assert!(project.contains("dev.example.unquoted"), "the fixture did not apply");
    fs::write(&project_path, project).unwrap();

    sync_project(&ios, "my_app", &Notes::new()).unwrap();

    let patched = read(&project_path);
    assert!(patched.contains("PRODUCT_BUNDLE_IDENTIFIER = \"dev.example.unquoted.LiveActivity\""), "{patched}");
    // The extension's own setting does not change what the app's is read as.
    assert_eq!(project_bundle_identifier(&patched).as_deref(), Some("dev.example.unquoted"));
}

#[test]
fn the_extension_keeps_its_bundle_name_whatever_the_product_name_override() {
    let ios = scaffolded("my_app");
    sync_project(&ios, "my_app", &Notes::new()).unwrap();

    let patched = read(ios.join("my_app.xcodeproj/project.pbxproj"));
    // `aimer assemble` passes PRODUCT_NAME on the command line; only WRAPPER_NAME survives it.
    assert_eq!(patched.matches("WRAPPER_NAME = \"my_appLiveActivity.appex\";").count(), 2, "{patched}");
}

#[test]
fn a_project_generated_before_the_bundle_name_was_pinned_is_repaired_once() {
    let ios = scaffolded("my_app");
    sync_project(&ios, "my_app", &Notes::new()).unwrap();
    let project_path = ios.join("my_app.xcodeproj/project.pbxproj");
    let old = read(&project_path).replace("\t\t\t\tWRAPPER_NAME = \"my_appLiveActivity.appex\";\n", "");
    assert!(!old.contains("WRAPPER_NAME"), "the fixture did not apply");
    fs::write(&project_path, &old).unwrap();

    let notes = Notes::new();
    sync_project(&ios, "my_app", &notes).unwrap();
    assert_eq!(read(&project_path).matches("WRAPPER_NAME").count(), 2);
    assert!(notes.all().iter().any(|n| n.contains("pinned")), "{:?}", notes.all());

    let again = Notes::new();
    sync_project(&ios, "my_app", &again).unwrap();
    assert!(again.all().is_empty(), "{:?}", again.all());
}
