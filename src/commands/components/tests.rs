use std::ffi::OsString;

use super::*;

fn args(cmd: &Command) -> Vec<String> {
    cmd.get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn trunk() -> &'static Component {
    catalog::find("trunk").unwrap()
}

fn ndk() -> &'static Component {
    catalog::find("ndk").unwrap()
}

fn selection(component: &'static Component, version: Option<&str>) -> Selection {
    Selection {
        component,
        version: version.map(str::to_string),
    }
}

/// A directory that `Ndk::at` accepts as an NDK of the given version.
fn fake_ndk(root: &Path, version: &str) {
    let dir = crate::commands::aimer_ndk::managed_dir(root).join(version);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("source.properties"),
        format!("Pkg.Revision = {version}\n"),
    )
    .unwrap();
}

#[test]
fn catalog_names_are_unique_and_installable() {
    for (index, component) in COMPONENTS.iter().enumerate() {
        if let Source::Cargo { package, binary } = component.source {
            assert!(!package.is_empty());
            assert!(!binary.is_empty());
        }
        assert!(
            COMPONENTS[index + 1..]
                .iter()
                .all(|other| other.name != component.name)
        );
    }
}

fn component(name: &str) -> &'static Component {
    catalog::find(name).unwrap()
}

#[test]
fn the_package_installer_generators_are_components() {
    assert_eq!(component("appimagetool").source, Source::AppImageTool);
    assert_eq!(component("inno-setup").source, Source::InnoSetup);
    assert_eq!(
        component("wix").source,
        Source::Dotnet {
            package: "wix",
            binary: "wix"
        }
    );
    assert_eq!(component("appimagetool").only_on, Some("linux"));
    assert_eq!(component("wix").only_on, Some("windows"));
}

#[test]
fn components_without_an_os_restriction_install_everywhere() {
    assert!(trunk().supported_here());
    assert!(ndk().supported_here());
}

#[test]
fn dotnet_tools_install_into_the_managed_bin() {
    let root = Path::new("/home/me/.aimer");
    assert_eq!(
        args(&dotnet_install_command(root, "wix")),
        [
            "tool",
            "install",
            "--tool-path",
            "/home/me/.aimer\\bin",
            "wix"
        ]
    );
    assert_eq!(
        args(&dotnet_uninstall_command(root, "wix")),
        [
            "tool",
            "uninstall",
            "--tool-path",
            "/home/me/.aimer/bin",
            "wix"
        ]
    );
}

#[test]
fn appimagetool_is_installed_when_its_binary_is_in_the_managed_bin() {
    let root = tempfile::tempdir().unwrap();
    let tool = component("appimagetool");
    assert!(!is_installed(root.path(), tool));
    std::fs::create_dir_all(bin_dir(root.path())).unwrap();
    std::fs::write(binary_path(root.path(), "appimagetool"), "").unwrap();
    assert!(is_installed(root.path(), tool));
}

#[test]
fn removing_appimagetool_deletes_the_binary() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(bin_dir(root.path())).unwrap();
    let path = binary_path(root.path(), "appimagetool");
    std::fs::write(&path, "").unwrap();
    remove(root.path(), &["appimagetool".to_string()]).unwrap();
    assert!(!path.exists());
}

#[test]
fn inno_setup_is_installed_when_iscc_is_in_its_folder() {
    let root = tempfile::tempdir().unwrap();
    let tool = component("inno-setup");
    assert!(!is_installed(root.path(), tool));
    std::fs::create_dir_all(packagers::inno_dir(root.path())).unwrap();
    std::fs::write(packagers::iscc_path(root.path()), "").unwrap();
    assert!(is_installed(root.path(), tool));
}

#[test]
fn installing_a_component_for_another_os_fails_clearly() {
    let root = tempfile::tempdir().unwrap();
    let other = if cfg!(windows) { "appimagetool" } else { "wix" };
    let error = install_one(root.path(), &selection(component(other), None), false)
        .unwrap_err()
        .to_string();
    assert!(error.contains("can only be installed on"), "{error}");
}

#[test]
fn resolve_rejects_a_version_for_the_installer_generators() {
    let error = resolve(&["wix@4".to_string()]).unwrap_err().to_string();
    assert!(error.contains("does not take a version"), "{error}");
}

#[test]
fn the_ndk_is_a_component() {
    assert_eq!(ndk().source, Source::Ndk);
}

#[test]
fn install_goes_into_the_aimer_root() {
    let cmd = install_command(Path::new("/home/me/.aimer"), "trunk");
    assert_eq!(
        args(&cmd),
        ["install", "--locked", "--root", "/home/me/.aimer", "trunk"]
    );
}

#[test]
fn uninstall_names_the_package() {
    let cmd = uninstall_command(Path::new("/home/me/.aimer"), "wasm-bindgen-cli");
    assert_eq!(
        args(&cmd),
        ["uninstall", "--root", "/home/me/.aimer", "wasm-bindgen-cli"]
    );
}

#[test]
fn resolve_rejects_unknown_names_and_lists_the_known_ones() {
    let error = resolve(&["nope".to_string()]).unwrap_err().to_string();
    assert!(error.contains("unknown component 'nope'"));
    assert!(error.contains("trunk"));
    assert!(error.contains("ndk"));
}

#[test]
fn resolve_drops_duplicates() {
    let names = ["trunk".to_string(), "trunk".to_string()];
    assert_eq!(resolve(&names).unwrap(), vec![selection(trunk(), None)]);
}

#[test]
fn resolve_reads_a_version_for_the_ndk() {
    let names = ["ndk@27".to_string(), "ndk".to_string()];
    assert_eq!(
        resolve(&names).unwrap(),
        vec![selection(ndk(), Some("27")), selection(ndk(), None)]
    );
}

#[test]
fn resolve_rejects_a_version_for_cargo_tools() {
    let error = resolve(&["trunk@1".to_string()]).unwrap_err().to_string();
    assert!(error.contains("does not take a version"));
}

#[test]
fn installed_means_the_binary_is_in_the_managed_bin() {
    let root = tempfile::tempdir().unwrap();
    assert!(!is_installed(root.path(), trunk()));
    std::fs::create_dir_all(bin_dir(root.path())).unwrap();
    std::fs::write(binary_path(root.path(), "trunk"), "").unwrap();
    assert!(is_installed(root.path(), trunk()));
}

#[test]
fn the_ndk_is_installed_when_a_managed_version_exists() {
    let root = tempfile::tempdir().unwrap();
    assert!(!is_installed(root.path(), ndk()));
    fake_ndk(root.path(), "27.3.13750724");
    assert!(is_installed(root.path(), ndk()));
}

#[test]
fn removing_a_component_that_is_not_installed_is_not_an_error() {
    let root = tempfile::tempdir().unwrap();
    remove(root.path(), &["trunk".to_string(), "ndk".to_string()]).unwrap();
}

#[test]
fn removing_an_unknown_component_fails() {
    let root = tempfile::tempdir().unwrap();
    assert!(remove(root.path(), &["nope".to_string()]).is_err());
}

#[test]
fn removing_the_ndk_deletes_every_managed_version() {
    let root = tempfile::tempdir().unwrap();
    fake_ndk(root.path(), "27.3.13750724");
    fake_ndk(root.path(), "29.0.14206865");
    remove(root.path(), &["ndk".to_string()]).unwrap();
    assert!(!is_installed(root.path(), ndk()));
}

#[test]
fn removing_a_versioned_ndk_keeps_the_others() {
    let root = tempfile::tempdir().unwrap();
    fake_ndk(root.path(), "27.3.13750724");
    fake_ndk(root.path(), "29.0.14206865");
    remove(root.path(), &["ndk@27".to_string()]).unwrap();
    assert_eq!(ndk::installed(root.path()), ["29.0.14206865"]);
}

#[test]
fn managed_bin_goes_last_on_the_path() {
    let bin = Path::new("/home/me/.aimer/bin");
    let path = env::join_paths(["/usr/bin", "/bin"]).unwrap();
    let joined = path_with_managed_bin(Some(path), bin);
    let entries: Vec<PathBuf> = env::split_paths(&joined).collect();
    assert_eq!(entries.last().unwrap(), bin);
    assert_eq!(entries.len(), 3);
}

#[test]
fn managed_bin_is_not_added_twice() {
    let bin = Path::new("/home/me/.aimer/bin");
    let path = env::join_paths(["/usr/bin", "/home/me/.aimer/bin"]).unwrap();
    let joined = path_with_managed_bin(Some(path.clone()), bin);
    assert_eq!(joined, path);
}

#[test]
fn managed_bin_alone_when_there_is_no_path() {
    let bin = Path::new("/home/me/.aimer/bin");
    assert_eq!(
        path_with_managed_bin(None, bin),
        OsString::from("/home/me/.aimer/bin")
    );
}
