use std::collections::HashMap;

use super::*;
use crate::commands::package::test_support::{SpyReporter, args_of};

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |key| map.get(key).cloned()
}

#[test]
fn appimagetool_is_taken_from_the_environment_when_set() {
    let env = env_of(&[(ENV_APPIMAGETOOL, "/opt/tools/appimagetool.AppImage")]);

    let found = locate_appimagetool(&env, &|p| {
        p == Path::new("/opt/tools/appimagetool.AppImage")
    });

    assert_eq!(
        found.unwrap(),
        Path::new("/opt/tools/appimagetool.AppImage")
    );
}

#[test]
fn a_wrong_environment_path_is_an_error_not_a_silent_search() {
    let env = env_of(&[(ENV_APPIMAGETOOL, "/nope"), ("PATH", "/usr/bin")]);

    let err = locate_appimagetool(&env, &|_| true)
        .unwrap_err()
        .to_string();

    assert!(err.contains("/nope"), "{err}");
}

#[test]
fn appimagetool_is_searched_on_path_by_both_of_its_names() {
    let env = env_of(&[("PATH", "/a:/b")]);

    let plain = locate_appimagetool(&env, &|p| p == Path::new("/b/appimagetool"));
    assert_eq!(plain.unwrap(), Path::new("/b/appimagetool"));

    let download = format!("/a/appimagetool-{}.AppImage", appimage_arch());
    let named = locate_appimagetool(&env, &|p| p == Path::new(&download));
    assert_eq!(named.unwrap(), Path::new(&download));
}

#[test]
fn a_missing_appimagetool_says_where_to_get_it_and_how_to_point_at_it() {
    let env = env_of(&[("PATH", "/usr/bin")]);

    let err = locate_appimagetool(&env, &|_| false)
        .unwrap_err()
        .to_string();

    assert!(err.contains(APPIMAGETOOL_URL), "{err}");
    assert!(err.contains(ENV_APPIMAGETOOL), "{err}");
}

#[test]
fn packages_are_named_after_app_version_and_architecture() {
    assert_eq!(
        output_name(PackageFormat::Appimage, "My App", "1.2.3", "x86_64"),
        "My App-1.2.3-x86_64.AppImage"
    );
    assert_eq!(
        output_name(PackageFormat::TarGz, "My App", "1.2.3", "aarch64"),
        "My App-1.2.3-linux-aarch64.tar.gz"
    );
    assert_eq!(
        output_name(PackageFormat::TarGz, "My App", "", "x86_64"),
        "My App-linux-x86_64.tar.gz"
    );
}

#[test]
fn the_architecture_uses_the_names_appimagetool_knows() {
    assert_eq!(arch_name("x86_64"), "x86_64");
    assert_eq!(arch_name("aarch64"), "aarch64");
    assert_eq!(arch_name("x86"), "i686");
    assert_eq!(arch_name("arm"), "armhf");
    assert_eq!(appimage_arch(), arch_name(std::env::consts::ARCH));
}

#[test]
fn the_launcher_runs_the_bundled_executable_with_its_arguments() {
    let script = app_run_script("my_app");

    assert!(script.starts_with("#!/bin/sh\n"), "{script}");
    assert!(script.contains("readlink -f"), "{script}");
    assert!(
        script.contains("exec \"$HERE/usr/bin/my_app\" \"$@\""),
        "{script}"
    );
}

#[test]
fn the_desktop_entry_points_at_the_launcher_and_the_appdir_icon() {
    let bundle_entry = "[Desktop Entry]\nName=My App\nExec=my_app\nIcon=app\nType=Application\n";

    let entry = appdir_desktop_entry(bundle_entry, "my_app");

    assert!(entry.contains("Exec=my_app\n"), "{entry}");
    assert!(entry.contains("Icon=my_app\n"), "{entry}");
    assert!(!entry.contains("Icon=app"), "{entry}");
    assert!(entry.contains("Name=My App\n"), "{entry}");
}

#[test]
fn a_bundle_without_a_desktop_entry_gets_a_minimal_one() {
    let entry = default_desktop_entry("my_app", "My App");

    assert!(entry.contains("Name=My App\n"), "{entry}");
    assert!(entry.contains("Exec=my_app\n"), "{entry}");
    assert!(entry.contains("Icon=my_app\n"), "{entry}");
    assert!(entry.contains("Type=Application\n"), "{entry}");
}

#[test]
fn tar_writes_the_archive_from_the_folder_inside_the_work_directory() {
    let cmd = tar_command(Path::new("/w"), "my_app-1.2.3", Path::new("/d/out.tar.gz"));

    assert_eq!(cmd.get_program(), "tar");
    assert_eq!(
        args_of(&cmd),
        ["-czf", "/d/out.tar.gz", "-C", "/w", "my_app-1.2.3"]
    );
}

#[test]
fn appimagetool_gets_the_appdir_the_output_and_the_architecture() {
    let cmd = appimagetool_command(
        Path::new("/t/appimagetool"),
        Path::new("/w/AppDir"),
        Path::new("/d/a.AppImage"),
    );

    assert_eq!(cmd.get_program(), "/t/appimagetool");
    assert_eq!(
        args_of(&cmd),
        ["--no-appstream", "/w/AppDir", "/d/a.AppImage"]
    );
    let env: HashMap<String, String> = cmd
        .get_envs()
        .filter_map(|(k, v)| Some((k.to_string_lossy().into(), v?.to_string_lossy().into())))
        .collect();
    assert_eq!(env["ARCH"], appimage_arch());
    assert_eq!(env["APPIMAGE_EXTRACT_AND_RUN"], "1");
}

/// A bundle as `aimer assemble linux` leaves it.
fn bundle(root: &Path, with_entry: bool, with_icon: bool) -> PathBuf {
    let bundle = root.join("bundle");
    std::fs::create_dir_all(bundle.join("assets/img")).unwrap();
    std::fs::write(bundle.join("my_app"), b"#!/bin/sh\n").unwrap();
    std::fs::write(bundle.join("assets/img/logo.png"), b"png").unwrap();
    if with_icon {
        std::fs::write(bundle.join("app.png"), b"icon").unwrap();
    }
    if with_entry {
        std::fs::write(
            bundle.join("my_app.desktop"),
            "[Desktop Entry]\nName=My App\nExec=my_app\nIcon=app\nType=Application\n",
        )
        .unwrap();
    }
    bundle
}

#[test]
fn the_appdir_holds_the_bundle_under_usr_bin_and_the_launch_files_at_the_top() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path(), true, true);
    let app_dir = dir.path().join("AppDir");

    build_app_dir(&bundle, &app_dir, "my_app", "My App").unwrap();

    assert!(app_dir.join("usr/bin/my_app").is_file());
    assert!(app_dir.join("usr/bin/assets/img/logo.png").is_file());
    assert!(app_dir.join("AppRun").is_file());
    assert_eq!(std::fs::read(app_dir.join("my_app.png")).unwrap(), b"icon");
    assert_eq!(std::fs::read(app_dir.join(".DirIcon")).unwrap(), b"icon");
    let entry = std::fs::read_to_string(app_dir.join("my_app.desktop")).unwrap();
    assert!(entry.contains("Icon=my_app\n"), "{entry}");
}

#[cfg(unix)]
#[test]
fn the_launcher_is_executable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path(), true, true);
    let app_dir = dir.path().join("AppDir");

    build_app_dir(&bundle, &app_dir, "my_app", "My App").unwrap();

    let mode = std::fs::metadata(app_dir.join("AppRun"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111);
}

#[test]
fn a_bundle_without_a_desktop_entry_still_builds_an_appdir() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path(), false, true);
    let app_dir = dir.path().join("AppDir");

    build_app_dir(&bundle, &app_dir, "my_app", "My App").unwrap();

    let entry = std::fs::read_to_string(app_dir.join("my_app.desktop")).unwrap();
    assert!(entry.contains("Name=My App\n"), "{entry}");
}

#[test]
fn a_bundle_without_an_icon_or_executable_is_rejected_with_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let no_icon = bundle(dir.path(), true, false);
    let err = build_app_dir(&no_icon, &dir.path().join("a"), "my_app", "My App")
        .unwrap_err()
        .to_string();
    assert!(err.contains("app.png"), "{err}");

    let err = build_app_dir(&no_icon, &dir.path().join("b"), "other", "My App")
        .unwrap_err()
        .to_string();
    assert!(err.contains("no executable 'other'"), "{err}");
}

fn job<'a>(
    format: PackageFormat,
    bundle: &'a Path,
    dist: &'a str,
    tool: Option<&'a Path>,
) -> LinuxJob<'a> {
    LinuxJob {
        format,
        pkg_name: "my_app",
        app_name: "My App",
        version: "1.2.3",
        bundle,
        dist,
        appimagetool: tool,
    }
}

/// A spy that writes the package a real tool would leave: the `tar` output
/// (the argument after `-czf`) or the `appimagetool` output (its last one).
fn writing_spy() -> SpyReporter {
    SpyReporter::with_hook(|cmd| {
        let args = args_of(cmd);
        let output = if cmd.get_program() == "tar" {
            args[1].clone()
        } else {
            args.last().unwrap().clone()
        };
        std::fs::write(output, b"package").unwrap();
    })
}

#[test]
fn a_tar_gz_is_staged_under_a_clean_folder_and_archived() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path(), true, true);
    let dist = dir.path().join("dist");
    let dist = dist.to_str().unwrap();
    let spy = writing_spy();

    let out = package_bundle(&job(PackageFormat::TarGz, &bundle, dist, None), &spy).unwrap();

    assert!(
        out.ends_with(&format!("My App-1.2.3-linux-{}.tar.gz", appimage_arch())),
        "{out}"
    );
    assert_eq!(spy.programs(), ["tar"]);
    let args = spy.steps()[0].args.clone();
    assert_eq!(args[2], "-C");
    assert_eq!(args[4], "my_app-1.2.3");
    assert!(!Path::new(dist).join(WORK_DIR_NAME).exists());
}

#[test]
fn an_appimage_is_assembled_and_handed_to_appimagetool() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path(), true, true);
    let dist = dir.path().join("dist");
    let dist = dist.to_str().unwrap();
    let tool = Path::new("/t/appimagetool");
    let spy = writing_spy();

    let out = package_bundle(
        &job(PackageFormat::Appimage, &bundle, dist, Some(tool)),
        &spy,
    )
    .unwrap();

    assert!(
        out.ends_with(&format!("My App-1.2.3-{}.AppImage", appimage_arch())),
        "{out}"
    );
    assert_eq!(spy.programs(), ["/t/appimagetool"]);
    assert!(spy.steps()[0].args[1].ends_with("AppDir"));
    assert!(!Path::new(dist).join(WORK_DIR_NAME).exists());
}

#[test]
fn an_appimage_without_a_tool_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path(), true, true);
    let dist = dir.path().join("dist");

    let err = package_bundle(
        &job(
            PackageFormat::Appimage,
            &bundle,
            dist.to_str().unwrap(),
            None,
        ),
        &SpyReporter::new(),
    )
    .unwrap_err();

    assert!(err.to_string().contains("appimagetool"), "{err}");
}

#[test]
fn a_tool_that_produces_nothing_is_reported_and_leaves_no_scratch_folder() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path(), true, true);
    let dist = dir.path().join("dist");
    let dist_str = dist.to_str().unwrap();

    let err = package_bundle(
        &job(PackageFormat::TarGz, &bundle, dist_str, None),
        &SpyReporter::new(),
    )
    .unwrap_err();

    assert!(err.to_string().contains("was not produced"), "{err}");
    assert!(!dist.join(WORK_DIR_NAME).exists());
}

#[test]
fn a_failing_tool_stops_the_package_and_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path(), true, true);
    let dist = dir.path().join("dist");
    let spy = SpyReporter::failing("tar");

    let err = package_bundle(
        &job(PackageFormat::TarGz, &bundle, dist.to_str().unwrap(), None),
        &spy,
    )
    .unwrap_err();

    assert!(err.to_string().contains("tar failed"), "{err}");
    assert!(!dist.join(WORK_DIR_NAME).exists());
}

#[test]
fn a_package_from_an_earlier_run_is_removed_first() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path(), true, true);
    let dist = dir.path().join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    let stale = dist.join(output_name(
        PackageFormat::TarGz,
        "My App",
        "1.2.3",
        appimage_arch(),
    ));
    std::fs::write(&stale, b"stale").unwrap();

    // The tool "succeeds" without writing anything.
    let err = package_bundle(
        &job(PackageFormat::TarGz, &bundle, dist.to_str().unwrap(), None),
        &SpyReporter::new(),
    )
    .unwrap_err();

    assert!(err.to_string().contains("was not produced"), "{err}");
    assert!(!stale.exists());
}
