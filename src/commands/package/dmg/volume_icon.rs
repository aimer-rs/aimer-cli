//! The icon of the mounted volume, which Finder also shows in the title bar of
//! the install window. Without one, the window shows the generic disk image
//! icon instead of the app's.
//!
//! A volume carries a custom icon when it holds a hidden `.VolumeIcon.icns`
//! and its root folder has the "custom icon" Finder flag set.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Name Finder looks for in the root of a volume.
pub(super) const VOLUME_ICON_FILE: &str = ".VolumeIcon.icns";

/// The `.icns` the app declares as its icon, falling back to the only (or
/// `AppIcon`) `.icns` in its resources.
pub(super) fn find_app_icon(app_path: &str) -> Option<PathBuf> {
    let contents = Path::new(app_path).join("Contents");
    let resources = contents.join("Resources");

    if let Some(name) = declared_icon_name(&contents.join("Info.plist")) {
        let name = if name.ends_with(".icns") {
            name
        } else {
            format!("{name}.icns")
        };
        let declared = resources.join(name);
        if declared.is_file() {
            return Some(declared);
        }
    }

    let mut icons: Vec<PathBuf> = std::fs::read_dir(&resources)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "icns"))
        .collect();
    icons.sort();
    icons
        .iter()
        .find(|path| path.file_name().is_some_and(|name| name == "AppIcon.icns"))
        .or(icons.first())
        .cloned()
}

/// `CFBundleIconFile` of an XML `Info.plist`; `None` for a binary one.
fn declared_icon_name(info_plist: &Path) -> Option<String> {
    let text = std::fs::read_to_string(info_plist).ok()?;
    let after_key = text.split_once("<key>CFBundleIconFile</key>")?.1;
    let value = after_key.split_once("<string>")?.1;
    let name = value.split_once("</string>")?.0.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Mark the mounted volume as having a custom icon.
pub(super) fn set_custom_icon_flag_command(volume_name: &str) -> Command {
    let mut cmd = Command::new("SetFile");
    cmd.args(["-a", "C"]).arg(format!("/Volumes/{volume_name}"));
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with(plist: Option<&str>, icons: &[&str]) -> tempfile::TempDir {
        let app = tempfile::tempdir().unwrap();
        let resources = app.path().join("Contents/Resources");
        std::fs::create_dir_all(&resources).unwrap();
        if let Some(plist) = plist {
            std::fs::write(app.path().join("Contents/Info.plist"), plist).unwrap();
        }
        for icon in icons {
            std::fs::write(resources.join(icon), b"icns").unwrap();
        }
        app
    }

    fn found(app: &tempfile::TempDir) -> Option<String> {
        find_app_icon(app.path().to_str().unwrap())
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
    }

    #[test]
    fn the_declared_icon_wins_with_or_without_extension() {
        let plist = "<dict><key>CFBundleIconFile</key><string>Forum</string></dict>";
        let app = app_with(Some(plist), &["AppIcon.icns", "Forum.icns"]);
        assert_eq!(found(&app).as_deref(), Some("Forum.icns"));

        let plist = "<dict><key>CFBundleIconFile</key><string>Forum.icns</string></dict>";
        let app = app_with(Some(plist), &["AppIcon.icns", "Forum.icns"]);
        assert_eq!(found(&app).as_deref(), Some("Forum.icns"));
    }

    #[test]
    fn without_a_declared_icon_app_icon_then_any_icon_is_used() {
        let app = app_with(None, &["Other.icns", "AppIcon.icns"]);
        assert_eq!(found(&app).as_deref(), Some("AppIcon.icns"));

        let app = app_with(None, &["Other.icns"]);
        assert_eq!(found(&app).as_deref(), Some("Other.icns"));
    }

    #[test]
    fn an_app_without_icons_has_none() {
        assert_eq!(found(&app_with(None, &[])), None);
        assert_eq!(find_app_icon("does/not/exist.app"), None);
    }

    #[test]
    fn the_flag_command_addresses_the_volume() {
        let cmd = set_custom_icon_flag_command("My App");
        assert_eq!(cmd.get_program(), "SetFile");
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["-a", "C", "/Volumes/My App"]);
    }
}
