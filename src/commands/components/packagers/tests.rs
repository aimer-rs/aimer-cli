use super::*;

fn args(cmd: &Command) -> Vec<String> {
    cmd.get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn appimagetool_is_downloaded_for_the_cpu_architecture() {
    assert_eq!(
        appimagetool_url("x86_64").unwrap(),
        "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
    );
    assert!(
        appimagetool_url("aarch64")
            .unwrap()
            .ends_with("appimagetool-aarch64.AppImage")
    );
    assert!(
        appimagetool_url("x86")
            .unwrap()
            .ends_with("appimagetool-i686.AppImage")
    );
    assert!(
        appimagetool_url("arm")
            .unwrap()
            .ends_with("appimagetool-armhf.AppImage")
    );
}

#[test]
fn appimagetool_has_no_download_for_an_unknown_architecture() {
    let error = appimagetool_url("riscv64").unwrap_err().to_string();
    assert!(error.contains("riscv64"), "{error}");
}

#[test]
fn inno_setup_lives_in_its_own_folder_of_the_root() {
    let root = Path::new("/home/me/.aimer");
    assert_eq!(inno_dir(root), Path::new("/home/me/.aimer/innosetup"));
    assert_eq!(
        iscc_path(root),
        Path::new("/home/me/.aimer/innosetup/ISCC.exe")
    );
}

#[test]
fn inno_setup_is_installed_silently_for_the_current_user() {
    let cmd = inno_install_command(Path::new("setup.exe"), Path::new("/r/innosetup"));
    assert_eq!(
        args(&cmd),
        [
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/CURRENTUSER",
            "/DIR=/r/innosetup"
        ]
    );
}

#[test]
fn removing_inno_setup_deletes_its_folder() {
    let root = tempfile::tempdir().unwrap();
    assert!(!remove_inno_setup(root.path()).unwrap());

    std::fs::create_dir_all(inno_dir(root.path())).unwrap();
    std::fs::write(iscc_path(root.path()), "").unwrap();
    assert!(remove_inno_setup(root.path()).unwrap());
    assert!(!inno_dir(root.path()).exists());
}
