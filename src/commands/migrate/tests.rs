use std::fs;

use super::*;

fn project<'a>(dir: &'a Path, force: bool) -> Project<'a> {
    Project {
        dir,
        name: "demo-app",
        app_name: "Demo App",
        group: "com.example.demo",
        force,
    }
}

#[test]
fn migrate_web_preserves_pkg_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Simulate an existing project with a web scaffold and a pkg/ dir.
    let web_dir = dir.join("builds/web");
    fs::create_dir_all(web_dir.join("pkg")).unwrap();
    fs::write(web_dir.join("pkg/artifact.wasm"), "wasm data").unwrap();

    migrate_platform(&project(dir, false), "web").unwrap();

    // Missing scaffold files are generated.
    let html = fs::read_to_string(web_dir.join("index.html")).unwrap();
    assert!(
        html.contains("data-trunk"),
        "index.html should be generated with Trunk attributes"
    );

    // Build artifacts are preserved.
    assert!(
        web_dir.join("pkg/artifact.wasm").exists(),
        "pkg/ artifacts should be preserved"
    );
}

#[test]
fn migrate_keeps_edited_scaffold_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    let web_dir = dir.join("builds/web");
    fs::create_dir_all(&web_dir).unwrap();
    fs::write(web_dir.join("index.html"), "<!-- my page -->").unwrap();
    fs::write(web_dir.join("notes.txt"), "mine").unwrap();

    migrate_platform(&project(dir, false), "web").unwrap();

    assert_eq!(
        fs::read_to_string(web_dir.join("index.html")).unwrap(),
        "<!-- my page -->",
        "a user-edited file must not be overwritten"
    );
    assert!(web_dir.join("Trunk.toml").exists(), "missing files return");
    assert!(
        web_dir.join("notes.txt").exists(),
        "files outside the scaffold must not be deleted"
    );
}

#[test]
fn migrate_force_replaces_edited_scaffold_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    let web_dir = dir.join("builds/web");
    fs::create_dir_all(&web_dir).unwrap();
    fs::write(web_dir.join("index.html"), "<!-- my page -->").unwrap();
    fs::write(web_dir.join("notes.txt"), "mine").unwrap();

    migrate_platform(&project(dir, true), "web").unwrap();

    let html = fs::read_to_string(web_dir.join("index.html")).unwrap();
    assert!(html.contains("data-trunk"), "{html}");
    assert!(web_dir.join("notes.txt").exists());
}

#[test]
fn migrate_platform_keeps_user_files_and_adds_the_scaffold() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Simulate an existing linux scaffold with a user's own file in it.
    let linux_dir = dir.join("builds/linux");
    fs::create_dir_all(&linux_dir).unwrap();
    fs::write(linux_dir.join("old_file.txt"), "old").unwrap();

    migrate_platform(&project(dir, false), "linux").unwrap();

    assert!(
        linux_dir.join("old_file.txt").exists(),
        "user files should be kept"
    );
    assert!(
        linux_dir.join("demo-app.desktop").exists(),
        "new scaffold should be generated"
    );
}

#[test]
fn migrate_ios_keeps_the_users_signing_settings_but_refreshes_the_bridge() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    migrate_platform(&project(dir, false), "ios").unwrap();
    let pbxproj = dir.join("builds/ios/demo-app.xcodeproj/project.pbxproj");
    let swift = dir.join("builds/ios/demo-app/main.swift");
    let plist = dir.join("builds/ios/demo-app/Info.plist");

    // The user signs the project and edits the plist; the bridge goes stale.
    let signed = format!(
        "{}\n// DEVELOPMENT_TEAM = ABCDE12345;\n",
        fs::read_to_string(&pbxproj).unwrap()
    );
    fs::write(&pbxproj, &signed).unwrap();
    fs::write(&plist, "<plist><!-- camera --></plist>").unwrap();
    fs::write(&swift, "// stale bridge").unwrap();

    migrate_platform(&project(dir, false), "ios").unwrap();

    assert_eq!(fs::read_to_string(&pbxproj).unwrap(), signed);
    assert_eq!(
        fs::read_to_string(&plist).unwrap(),
        "<plist><!-- camera --></plist>"
    );
    assert!(
        fs::read_to_string(&swift)
            .unwrap()
            .contains("aimer_ios_sync_text_state"),
        "generated code is brought up to date"
    );
}

#[test]
fn migrate_mobile_scaffolds_installs_revisioned_text_mirrors() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let project = Project {
        name: "demo_app",
        app_name: "demo_app",
        ..project(dir, false)
    };

    migrate_platform(&project, "ios").unwrap();
    migrate_platform(&project, "android").unwrap();

    let swift = fs::read_to_string(dir.join("builds/ios/demo_app/main.swift")).unwrap();
    let kotlin = fs::read_to_string(
        dir.join("builds/android/app/src/main/kotlin/com/aimer/AimerActivity.kt"),
    )
    .unwrap();
    assert!(swift.contains("aimer_ios_sync_text_state"));
    assert!(swift.contains("trigger_rust_text_editing_delta"));
    assert!(kotlin.contains("fun syncTextState"));
    assert!(kotlin.contains("nativeTextEditingDelta"));
}

#[test]
fn migrate_rejects_unknown_target() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::write(
        dir.join("aimer.toml"),
        "[application]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let result = execute_in("playstation".to_string(), dir, false);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("unknown target"));
}

#[test]
fn migrate_requires_aimer_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let result = execute_in("web".to_string(), tmp.path(), false);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("no Aimer.toml"));
}
