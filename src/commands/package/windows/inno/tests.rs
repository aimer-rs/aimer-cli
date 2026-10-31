use std::path::PathBuf;

use super::*;
use crate::commands::package::test_support::args_of;

fn installer<'a>(bundle: &'a Path, dist: &'a Path) -> Installer<'a> {
    Installer {
        app_name: "My App",
        exe_name: "my_app.exe",
        version: "1.2.3",
        publisher: "Jane Doe",
        bundle,
        dist,
        base_name: "My App-1.2.3-x64-setup",
        app_id: "8E3F1A6B-1111-5222-8333-444455556666",
        arch: "x64",
    }
}

fn script_of(edit: impl FnOnce(&mut Installer)) -> String {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("app.ico"), b"ico").unwrap();
    let bundle = dir.path().to_path_buf();
    let dist = PathBuf::from("/out/dist");
    let mut info = installer(&bundle, &dist);
    edit(&mut info);
    script(&info)
}

fn line<'a>(script: &'a str, key: &str) -> Option<&'a str> {
    script.lines().find_map(|l| l.strip_prefix(key))
}

#[test]
fn iscc_is_run_quietly_on_the_script() {
    let cmd = command(Path::new("C:/Inno/ISCC.exe"), Path::new("C:/w/setup.iss"));

    assert_eq!(cmd.get_program(), "C:/Inno/ISCC.exe");
    assert_eq!(args_of(&cmd), ["/Q", "C:/w/setup.iss"]);
}

#[test]
fn the_setup_section_names_the_app_the_output_and_the_stable_id() {
    let script = script_of(|_| {});

    assert_eq!(
        line(&script, "AppId="),
        Some("{{8E3F1A6B-1111-5222-8333-444455556666}")
    );
    assert_eq!(line(&script, "AppName="), Some("My App"));
    assert_eq!(line(&script, "AppVersion="), Some("1.2.3"));
    assert_eq!(line(&script, "AppPublisher="), Some("Jane Doe"));
    assert_eq!(line(&script, "DefaultDirName="), Some("{autopf}\\My App"));
    assert_eq!(line(&script, "OutputDir="), Some("/out/dist"));
    assert_eq!(
        line(&script, "OutputBaseFilename="),
        Some("My App-1.2.3-x64-setup")
    );
    assert_eq!(
        line(&script, "UninstallDisplayIcon="),
        Some("{app}\\my_app.exe")
    );
}

#[test]
fn it_installs_per_user_without_a_prompt_but_lets_the_user_choose_all_users() {
    let script = script_of(|_| {});

    assert_eq!(line(&script, "PrivilegesRequired="), Some("lowest"));
    assert_eq!(
        line(&script, "PrivilegesRequiredOverridesAllowed="),
        Some("dialog")
    );
}

#[test]
fn the_whole_bundle_is_installed_with_its_folders() {
    let dir = tempfile::tempdir().unwrap();
    let dist = PathBuf::from("/out");
    let script = script(&installer(dir.path(), &dist));

    assert!(
        script.contains(&format!(
            "Source: \"{}\\*\"; DestDir: \"{{app}}\"; Flags: ignoreversion recursesubdirs createallsubdirs",
            dir.path().display()
        )),
        "{script}"
    );
}

#[test]
fn there_is_a_start_menu_entry_an_optional_desktop_shortcut_and_a_launch_checkbox() {
    let script = script_of(|_| {});

    assert!(
        script.contains("Name: \"{autoprograms}\\My App\"; Filename: \"{app}\\my_app.exe\"\n"),
        "{script}"
    );
    assert!(
        script.contains(
            "Name: \"{autodesktop}\\My App\"; Filename: \"{app}\\my_app.exe\"; Tasks: desktopicon"
        ),
        "{script}"
    );
    assert!(
        script.contains("[Tasks]\nName: \"desktopicon\""),
        "{script}"
    );
    assert!(script.contains("Flags: unchecked"), "{script}");
    assert!(
        script.contains("[Run]\nFilename: \"{app}\\my_app.exe\"; Description: \"Launch My App\"; Flags: nowait postinstall skipifsilent"),
        "{script}"
    );
}

#[test]
fn the_setup_program_gets_the_app_icon_when_the_bundle_has_one() {
    let with_icon = script_of(|_| {});
    assert!(line(&with_icon, "SetupIconFile=").is_some_and(|p| p.ends_with("app.ico")));

    let dir = tempfile::tempdir().unwrap();
    let dist = PathBuf::from("/out");
    let without = script(&installer(dir.path(), &dist));
    assert!(line(&without, "SetupIconFile=").is_none());
}

#[test]
fn the_architecture_decides_how_it_installs() {
    let x64 = script_of(|i| i.arch = "x64");
    assert_eq!(line(&x64, "ArchitecturesAllowed="), Some("x64compatible"));
    assert_eq!(
        line(&x64, "ArchitecturesInstallIn64BitMode="),
        Some("x64compatible")
    );

    let arm = script_of(|i| i.arch = "arm64");
    assert_eq!(line(&arm, "ArchitecturesAllowed="), Some("arm64"));

    let x86 = script_of(|i| i.arch = "x86");
    assert!(line(&x86, "ArchitecturesAllowed=").is_none());
}

#[test]
fn a_missing_version_or_publisher_is_handled() {
    let script = script_of(|i| {
        i.version = "  ";
        i.publisher = "";
    });

    assert_eq!(line(&script, "AppVersion="), Some("1.0.0"));
    assert!(line(&script, "AppPublisher=").is_none());
}

#[test]
fn characters_with_a_meaning_to_the_compiler_are_escaped() {
    let script = script_of(|i| i.app_name = "My {App} \"Pro\"");

    // Only an opening brace starts a constant, so only it is doubled; a closing
    // one is literal. In a quoted parameter a double quote is doubled as well.
    assert_eq!(line(&script, "AppName="), Some("My {{App} \"Pro\""));
    // A shortcut is a file name, and `"` is not allowed in one, so it is dropped
    // there; the brace is still doubled.
    assert!(
        script.contains("Name: \"{autoprograms}\\My {{App} Pro\"; Filename: "),
        "{script}"
    );
    assert!(
        script.contains("Description: \"Launch My {{App} \"\"Pro\"\"\""),
        "{script}"
    );
}

#[test]
fn the_install_folder_never_contains_a_character_windows_forbids() {
    let script = script_of(|i| i.app_name = "A/B: C*D?");

    assert_eq!(line(&script, "DefaultDirName="), Some("{autopf}\\AB CD"));
    assert_eq!(folder_name("???"), "App");
    assert_eq!(folder_name("Name."), "Name");
}
