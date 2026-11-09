use super::*;
use crate::commands::package::test_support::{SpyReporter, args_of};

/// A volume name nothing would mount, so the tests never depend on `/Volumes`.
const APP: &str = "AimerDmgTestApp";

fn request<'a>(dist: &'a str, output: &'a str, tool: ImageTool, mounted: bool) -> DmgRequest<'a> {
    DmgRequest {
        app_path: "builds/macos/build/Release/AimerDmgTestApp.app",
        app_name: APP,
        dist,
        output,
        tool,
        volume_mounted: mounted,
    }
}

/// Build with a spy and return it together with the scratch directory.
fn build(tool: ImageTool, mounted: bool, spy: SpyReporter) -> (SpyReporter, tempfile::TempDir) {
    let dist = tempfile::tempdir().unwrap();
    let dist_path = dist.path().to_str().unwrap().to_string();
    let output = format!("{dist_path}/App.dmg");
    build_dmg(&request(&dist_path, &output, tool, mounted), &spy).unwrap();
    (spy, dist)
}

fn actions(spy: &SpyReporter) -> Vec<String> {
    spy.steps().into_iter().map(|s| s.action).collect()
}

#[test]
fn diskutil_creates_an_image_of_a_folder_with_its_volume_name() {
    let cmd = diskutil_create_command("UDZO", Some("My App"), "stage", "out/My App.dmg");

    assert_eq!(cmd.get_program(), "diskutil");
    assert_eq!(
        args_of(&cmd),
        [
            "image",
            "create",
            "from",
            "--format",
            "UDZO",
            "--volumeName",
            "My App",
            "stage",
            "out/My App.dmg",
        ]
    );
}

#[test]
fn converting_an_image_leaves_the_volume_name_alone() {
    let cmd = diskutil_create_command("UDZO", None, "work/w.sparsebundle", "App.dmg");

    assert_eq!(
        args_of(&cmd),
        [
            "image",
            "create",
            "from",
            "--format",
            "UDZO",
            "work/w.sparsebundle",
            "App.dmg"
        ]
    );
}

#[test]
fn attach_and_eject_address_the_volume() {
    assert_eq!(
        args_of(&diskutil_attach_command("w.sparsebundle")),
        ["image", "attach", "w.sparsebundle"]
    );
    let eject = diskutil_eject_command("My App");
    assert_eq!(eject.get_program(), "diskutil");
    assert_eq!(args_of(&eject), ["eject", "/Volumes/My App"]);
}

#[test]
fn the_hdiutil_fallback_creates_a_compressed_image() {
    let cmd = hdiutil_command("My App", "stage", "My App.dmg");

    assert_eq!(cmd.get_program(), "hdiutil");
    assert_eq!(
        args_of(&cmd),
        [
            "create",
            "-volname",
            "My App",
            "-srcfolder",
            "stage",
            "-ov",
            "-format",
            "UDZO",
            "My App.dmg",
        ]
    );
}

#[test]
fn staging_copies_the_app_and_links_applications() {
    assert_eq!(
        args_of(&ditto_command("a/My App.app", "stage/My App.app")),
        ["a/My App.app", "stage/My App.app"]
    );
    assert_eq!(
        args_of(&applications_link_command("stage")),
        ["-s", "/Applications", "stage/Applications"]
    );
}

#[test]
fn the_layout_script_gets_the_window_geometry() {
    let cmd = layout_command("layout.applescript", "My App");

    assert_eq!(cmd.get_program(), "osascript");
    assert_eq!(
        args_of(&cmd),
        [
            "layout.applescript",
            "My App",
            "My App.app",
            "660",
            "400",
            "170",
            "490",
            "190"
        ]
    );
}

#[test]
fn the_background_script_gets_the_icon_to_take_its_colors_from() {
    let cmd = background_command("bg.js", "out.png", "My App", "/abs/My App.app");

    assert_eq!(
        args_of(&cmd),
        [
            "-l",
            "JavaScript",
            "bg.js",
            "out.png",
            "My App",
            "660",
            "400",
            "330",
            "190",
            "/abs/My App.app"
        ]
    );
}

#[test]
fn the_chevrons_sit_between_the_two_icons() {
    assert_eq!(CHEVRON_X, 330);
    assert!(APP_ICON_X < CHEVRON_X && CHEVRON_X < APPLICATIONS_ICON_X);
    assert!(APPLICATIONS_ICON_X < WINDOW_WIDTH && ICON_Y < WINDOW_HEIGHT);
}

#[test]
fn the_embedded_scripts_are_the_ones_the_commands_expect() {
    assert!(BACKGROUND_SCRIPT.contains("iconForFile"));
    assert!(LAYOUT_SCRIPT.contains("on run argv"));
    assert!(LAYOUT_SCRIPT.contains(".background:background.png"));
}

#[test]
fn with_diskutil_the_window_is_styled_through_a_writable_image() {
    let (spy, _dist) = build(ImageTool::Diskutil, false, SpyReporter::new());

    assert_eq!(
        actions(&spy),
        [
            "ditto",
            "ln",
            "disk image background",
            "diskutil image create (writable)",
            "diskutil image attach",
            "disk image layout",
            "diskutil eject",
            "diskutil image create",
        ]
    );
    let steps = spy.steps();
    assert!(steps[3].args.contains(&"UDSB".to_string()));
    assert!(steps[7].args.contains(&"UDZO".to_string()));
}

#[test]
fn the_scratch_folders_are_removed_afterwards() {
    let (_spy, dist) = build(ImageTool::Diskutil, false, SpyReporter::new());

    assert!(!dist.path().join(STAGING_DIR_NAME).exists());
    assert!(!dist.path().join(WORK_DIR_NAME).exists());
}

#[test]
fn a_failed_layout_still_ejects_and_falls_back_to_a_plain_image() {
    let (spy, dist) = build(
        ImageTool::Diskutil,
        false,
        SpyReporter::failing("disk image layout"),
    );

    let actions = actions(&spy);
    let layout = actions
        .iter()
        .position(|a| a == "disk image layout")
        .unwrap();
    assert_eq!(actions[layout + 1], "diskutil eject");
    assert_eq!(actions.last().unwrap(), "diskutil image create");
    let plain = spy.steps().pop().unwrap();
    assert!(plain.args.contains(&"--volumeName".to_string()));
    assert!(spy.notes().iter().any(|n| n.starts_with("warning:")));
    assert!(!dist.path().join(STAGING_DIR_NAME).exists());
}

#[test]
fn a_failed_background_does_not_touch_any_volume() {
    let (spy, _dist) = build(
        ImageTool::Diskutil,
        false,
        SpyReporter::failing("disk image background"),
    );

    let actions = actions(&spy);
    assert!(!actions.contains(&"diskutil image attach".to_string()));
    assert!(!actions.contains(&"diskutil eject".to_string()));
}

#[test]
fn a_volume_of_the_same_name_is_never_attached_over_or_ejected() {
    let (spy, _dist) = build(ImageTool::Diskutil, true, SpyReporter::new());

    assert_eq!(actions(&spy), ["ditto", "ln", "diskutil image create"]);
    assert!(
        spy.notes()
            .iter()
            .any(|n| n.contains("already mounted") && n.starts_with("warning:"))
    );
}

#[test]
fn without_diskutil_image_a_plain_hdiutil_image_is_made() {
    let (spy, _dist) = build(ImageTool::Hdiutil, false, SpyReporter::new());

    assert_eq!(spy.programs(), ["ditto", "ln", "hdiutil"]);
}

#[test]
fn the_staging_failure_leaves_no_scratch_folder_behind() {
    let dist = tempfile::tempdir().unwrap();
    let dist_path = dist.path().to_str().unwrap();
    let spy = SpyReporter::failing("ditto");

    let result = build_dmg(
        &request(
            dist_path,
            &format!("{dist_path}/App.dmg"),
            ImageTool::Diskutil,
            false,
        ),
        &spy,
    );

    assert!(result.is_err());
    assert!(!dist.path().join(STAGING_DIR_NAME).exists());
    assert!(!dist.path().join(WORK_DIR_NAME).exists());
}

#[test]
fn an_app_icon_becomes_the_volume_icon() {
    let app = tempfile::tempdir().unwrap();
    let resources = app.path().join("Contents/Resources");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(resources.join("AppIcon.icns"), b"icns").unwrap();
    let dist = tempfile::tempdir().unwrap();
    let dist_path = dist.path().to_str().unwrap().to_string();
    let output = format!("{dist_path}/App.dmg");
    let app_path = app.path().to_str().unwrap();
    let request = DmgRequest {
        app_path,
        ..request(&dist_path, &output, ImageTool::Diskutil, false)
    };
    let spy = SpyReporter::new();

    build_dmg(&request, &spy).unwrap();

    assert_eq!(
        actions(&spy),
        [
            "ditto",
            "ln",
            "disk image background",
            "diskutil image create (writable)",
            "diskutil image attach",
            "SetFile",
            "disk image layout",
            "diskutil eject",
            "diskutil image create",
        ]
    );
}
