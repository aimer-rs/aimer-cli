use super::*;
use crate::commands::package::test_support::args_of;

const CODE: &str = "8E3F1A6B-1111-5222-8333-444455556666";

#[test]
fn an_msi_version_is_major_minor_build() {
    assert_eq!(msi_version("1.2.3").unwrap(), "1.2.3");
    assert_eq!(msi_version("0.1.0").unwrap(), "0.1.0");
    assert_eq!(msi_version("2").unwrap(), "2.0.0");
    assert_eq!(msi_version("1.2").unwrap(), "1.2.0");
    assert_eq!(msi_version("1.2.3-beta+7").unwrap(), "1.2.3");
    assert_eq!(msi_version("255.255.65535").unwrap(), "255.255.65535");
    assert_eq!(msi_version("").unwrap(), "1.0.0");
    assert_eq!(msi_version("  ").unwrap(), "1.0.0");
}

#[test]
fn a_version_an_msi_cannot_carry_is_rejected_with_the_limits() {
    for version in [
        "256.0.0",
        "1.256.0",
        "1.0.65536",
        "1.2.3.4",
        "x.y.z",
        "1..2",
        "v1.0.0",
    ] {
        let err = msi_version(version).unwrap_err().to_string();
        assert!(
            err.contains("MSI version") || err.contains("more than three"),
            "{version}: {err}"
        );
    }
}

#[test]
fn wix_builds_for_the_architecture_without_leaving_a_pdb_behind() {
    let cmd = command(
        Path::new("C:/dotnet/wix.exe"),
        Path::new("C:/w/package.wxs"),
        Path::new("C:/d/App.msi"),
        "x64",
    );

    assert_eq!(cmd.get_program(), "C:/dotnet/wix.exe");
    assert_eq!(
        args_of(&cmd),
        [
            "build",
            "-arch",
            "x64",
            "-pdbtype",
            "none",
            "-o",
            "C:/d/App.msi",
            "C:/w/package.wxs",
        ]
    );
}

fn bundle(root: &Path) -> std::path::PathBuf {
    let bundle = root.join("bundle");
    std::fs::create_dir_all(bundle.join("assets/img")).unwrap();
    std::fs::write(bundle.join("my_app.exe"), b"MZ").unwrap();
    std::fs::write(bundle.join("assets/data.json"), b"{}").unwrap();
    std::fs::write(bundle.join("assets/img/logo.png"), b"png").unwrap();
    bundle
}

fn installer<'a>(bundle: &'a Path) -> Installer<'a> {
    Installer {
        app_name: "My App",
        exe_name: "my_app.exe",
        version: "1.2.3",
        publisher: "Jane Doe",
        bundle,
        upgrade_code: CODE,
    }
}

fn count(text: &str, needle: &str) -> usize {
    text.matches(needle).count()
}

#[test]
fn the_package_names_the_product_version_and_a_stable_upgrade_code() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());

    let xml = source(&installer(&bundle)).unwrap();

    assert!(
        xml.contains("xmlns=\"http://wixtoolset.org/schemas/v4/wxs\""),
        "{xml}"
    );
    assert!(xml.contains("Name=\"My App\""), "{xml}");
    assert!(xml.contains("Manufacturer=\"Jane Doe\""), "{xml}");
    assert!(xml.contains("Version=\"1.2.3\""), "{xml}");
    assert!(xml.contains(&format!("UpgradeCode=\"{CODE}\"")), "{xml}");
    assert!(xml.contains("Scope=\"perMachine\""), "{xml}");
    assert!(
        xml.contains("<MajorUpgrade DowngradeErrorMessage="),
        "{xml}"
    );
    assert!(xml.contains("<MediaTemplate EmbedCab=\"yes\" />"), "{xml}");
}

#[test]
fn the_manufacturer_falls_back_to_the_app_name() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let mut info = installer(&bundle);
    info.publisher = "  ";

    let xml = source(&info).unwrap();

    assert!(xml.contains("Manufacturer=\"My App\""), "{xml}");
}

#[test]
fn every_file_is_a_component_and_every_folder_a_directory() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());

    let xml = source(&installer(&bundle)).unwrap();

    // Three files plus the Start menu entry.
    assert_eq!(count(&xml, "<Component "), 4, "{xml}");
    assert_eq!(count(&xml, "</Component>"), 4, "{xml}");
    assert_eq!(count(&xml, "<File "), 3, "{xml}");
    // INSTALLFOLDER and ApplicationProgramsFolder, plus `assets` and `img`.
    assert_eq!(count(&xml, "<Directory "), 4, "{xml}");
    assert_eq!(count(&xml, "</Directory>"), 4, "{xml}");
    assert_eq!(
        count(&xml, "<StandardDirectory "),
        count(&xml, "</StandardDirectory>")
    );
    assert!(
        xml.contains("<Directory Id=\"D1\" Name=\"assets\">"),
        "{xml}"
    );
    assert!(xml.contains("<Directory Id=\"D2\" Name=\"img\">"), "{xml}");
    assert!(xml.contains("my_app.exe"), "{xml}");
}

#[test]
fn every_component_is_installed_by_the_feature() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());

    let xml = source(&installer(&bundle)).unwrap();

    for id in ["C1", "C2", "C3", "StartMenuShortcut"] {
        assert!(
            xml.contains(&format!("<ComponentRef Id=\"{id}\" />")),
            "{id}\n{xml}"
        );
    }
    assert_eq!(count(&xml, "<ComponentRef "), 4, "{xml}");
}

#[test]
fn ids_are_unique_and_valid_identifiers() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let xml = source(&installer(&bundle)).unwrap();

    // A `ComponentRef` repeats an id on purpose; every other `Id` defines one.
    let defined: Vec<&str> = xml
        .lines()
        .filter(|line| !line.contains("<ComponentRef"))
        .flat_map(|line| line.split("Id=\"").skip(1))
        .filter_map(|rest| rest.split('"').next())
        .collect();
    assert!(defined.len() > 8, "{defined:?}");
    let mut seen = std::collections::HashSet::new();
    for id in defined {
        assert!(seen.insert(id), "duplicate id {id}\n{xml}");
        assert!(
            id.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.'),
            "{id}"
        );
    }
}

#[test]
fn the_start_menu_shortcut_opens_the_executable_and_is_removed_on_uninstall() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());

    let xml = source(&installer(&bundle)).unwrap();

    assert!(
        xml.contains(
            "<Shortcut Id=\"AppShortcut\" Name=\"My App\" Target=\"[INSTALLFOLDER]my_app.exe\""
        ),
        "{xml}"
    );
    assert!(
        xml.contains("<RemoveFolder Id=\"RemoveProgramsFolder\" On=\"uninstall\" />"),
        "{xml}"
    );
    assert!(xml.contains("<RegistryValue Root=\"HKCU\""), "{xml}");
    assert!(xml.contains("KeyPath=\"yes\""), "{xml}");
}

#[test]
fn the_add_remove_programs_icon_is_set_when_the_bundle_has_one() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    assert!(
        !source(&installer(&bundle))
            .unwrap()
            .contains("ARPPRODUCTICON")
    );

    std::fs::write(bundle.join("app.ico"), b"ico").unwrap();
    let xml = source(&installer(&bundle)).unwrap();
    assert!(xml.contains("<Icon Id=\"AppIcon.ico\""), "{xml}");
    assert!(
        xml.contains("<Property Id=\"ARPPRODUCTICON\" Value=\"AppIcon.ico\" />"),
        "{xml}"
    );
}

#[test]
fn text_is_escaped_for_xml() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let mut info = installer(&bundle);
    info.app_name = "Tom & Jerry <\"1\">";
    info.publisher = "O'Neil";

    let xml = source(&info).unwrap();

    assert!(
        xml.contains("Name=\"Tom &amp; Jerry &lt;&quot;1&quot;&gt;\""),
        "{xml}"
    );
    assert!(xml.contains("Manufacturer=\"O&apos;Neil\""), "{xml}");
    assert!(!xml.contains("Tom & Jerry"), "{xml}");
}

#[test]
fn a_version_that_does_not_fit_fails_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let mut info = installer(&bundle);
    info.version = "1.2.3.4";

    assert!(source(&info).is_err());
}

#[test]
fn a_missing_bundle_is_an_error() {
    let dir = tempfile::tempdir().unwrap();

    assert!(source(&installer(&dir.path().join("nope"))).is_err());
}
