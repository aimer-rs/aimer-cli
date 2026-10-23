use std::sync::Mutex;

use super::*;

/// A [`Reporter`] that records the notes and the steps it was asked to run
/// instead of touching the machine.
struct SpyReporter {
    notes: Mutex<Vec<String>>,
    steps: Mutex<Vec<(Step, Vec<String>)>>,
}

impl SpyReporter {
    fn new() -> Self {
        Self {
            notes: Mutex::new(Vec::new()),
            steps: Mutex::new(Vec::new()),
        }
    }
}

impl Reporter for SpyReporter {
    fn note(&self, message: String) {
        self.notes.lock().unwrap().push(message);
    }

    fn run(&self, cmd: Command, step: Step) -> anyhow::Result<()> {
        self.steps.lock().unwrap().push((step, args_of(&cmd)));
        Ok(())
    }

    fn stage_assets(&self, dest_root: &str) -> anyhow::Result<()> {
        self.note(format!("staged {dest_root}"));
        Ok(())
    }
}

/// The arguments of `cmd`, as lossy strings.
fn args_of(cmd: &Command) -> Vec<String> {
    cmd.get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect()
}

#[test]
fn profile_name_maps_release_flag() {
    assert_eq!(profile_name(true), "release");
    assert_eq!(profile_name(false), "debug");
}

#[test]
fn xcode_configuration_maps_release_flag() {
    assert_eq!(xcode_configuration(true), "Release");
    assert_eq!(xcode_configuration(false), "Debug");
}

#[test]
fn only_jdk_17_and_newer_are_considered() {
    assert!(!SUPPORTED_JDK_VERSIONS.is_empty());
    for version in SUPPORTED_JDK_VERSIONS {
        let feature: u32 = version.parse().expect("JDK feature release");
        assert!(feature >= 17, "Gradle 9 rejects JDK {version}");
    }
}

#[test]
fn gradle_task_maps_release_flag() {
    assert_eq!(gradle_task(true), "assembleRelease");
    assert_eq!(gradle_task(false), "assembleDebug");
}

#[test]
fn artifact_path_uses_profile_dir() {
    let debug = artifact_path("aarch64-apple-darwin", "my_app", false, ".a");
    let release = artifact_path("aarch64-apple-darwin", "my_app", true, ".a");
    assert!(debug.ends_with("/target/aarch64-apple-darwin/debug/libmy_app.a"));
    assert!(release.ends_with("/target/aarch64-apple-darwin/release/libmy_app.a"));
}

#[test]
fn lib_name_replaces_dashes() {
    assert_eq!(lib_name_of("my-cool-app"), "my_cool_app");
}

#[test]
fn macos_app_path_follows_the_configuration() {
    assert_eq!(
        macos_app_path("my_app", false),
        "builds/macos/build/Debug/my_app.app"
    );
    assert_eq!(
        macos_app_path("my_app", true),
        "builds/macos/build/Release/my_app.app"
    );
}

// ── Desktop bundles ──────────────────────────────────────────────

#[test]
fn desktop_bundle_path_follows_the_platform_and_profile() {
    assert_eq!(
        desktop_bundle_path(Targets::Linux, "my_app", false),
        "builds/linux/bundle/debug/my_app"
    );
    assert_eq!(
        desktop_bundle_path(Targets::Windows, "my_app", true),
        "builds/windows/bundle/release/my_app"
    );
}

#[test]
fn desktop_exe_path_keeps_the_package_name_verbatim() {
    // Cargo names the default binary after the package, dashes included.
    let proj_root = get_project_root(true).unwrap_or_default();
    assert_eq!(
        desktop_exe_path("my-cool-app", false),
        proj_root.join(format!(
            "target/debug/my-cool-app{}",
            std::env::consts::EXE_SUFFIX
        ))
    );
    assert_eq!(
        desktop_exe_path("my-cool-app", true),
        proj_root.join(format!(
            "target/release/my-cool-app{}",
            std::env::consts::EXE_SUFFIX
        ))
    );
}

/// A project tree whose compiled binary sits where [`desktop_exe_path`]
/// resolves it.
///
/// [`desktop_exe_path`] anchors the executable at the workspace root (via
/// `cargo locate-project`), so the fake binary is staged into the real
/// `target/` rather than under the per-test temp dir, which only holds the
/// bundle layout [`package_desktop_in`] lays out.
fn desktop_project(pkg_name: &str) -> tempfile::TempDir {
    // Several tests stage the same binary name, so write it under a lock —
    // a parallel test must never read a half-written file.
    static STAGE_EXECUTABLE: Mutex<()> = Mutex::new(());
    let _guard = STAGE_EXECUTABLE.lock().unwrap();

    let exe = PathBuf::from(desktop_exe_path(pkg_name, false));
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"#!/bin/sh\ntrue\n").unwrap();
    tempfile::tempdir().expect("temp project")
}

/// Write the Linux scaffold `create::linux` generates for `pkg_name`.
fn linux_scaffold(root: &Path, pkg_name: &str) {
    let dir = root.join("builds/linux");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{pkg_name}.desktop")),
        "[Desktop Entry]\nName=Demo\nExec=aimer_app\nIcon=aimer\n",
    )
    .unwrap();
    std::fs::write(dir.join("app.png"), b"png").unwrap();
}

#[test]
fn packaging_stages_the_executable_and_the_linux_scaffold() {
    let pkg = "my_app";
    let project = desktop_project(pkg);
    linux_scaffold(project.path(), pkg);
    let reporter = SpyReporter::new();

    let bundle = package_desktop_in(project.path(), Targets::Linux, pkg, false, &reporter).unwrap();

    assert_eq!(bundle, "builds/linux/bundle/debug/my_app");
    let dir = project.path().join(&bundle);
    assert!(dir.join(desktop_exe_name(pkg)).is_file());
    assert!(dir.join("app.png").is_file());
    let entry = std::fs::read_to_string(dir.join("my_app.desktop")).unwrap();
    assert!(entry.contains("Exec=my_app"), "{entry}");
    assert!(!entry.contains("aimer_app"), "{entry}");
    assert!(entry.contains("Name=Demo"), "{entry}");
    // The assets land next to the executable, where the runtime looks.
    assert!(
        reporter
            .notes
            .lock()
            .unwrap()
            .contains(&format!("staged {bundle}"))
    );
}

#[test]
fn packaging_stages_the_windows_scaffold_next_to_the_executable() {
    let pkg = "my_app";
    let project = desktop_project(pkg);
    let dir = project.path().join("builds/windows");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("app.ico"), b"ico").unwrap();
    std::fs::write(dir.join("app.manifest"), b"<assembly/>").unwrap();
    let reporter = SpyReporter::new();

    let bundle =
        package_desktop_in(project.path(), Targets::Windows, pkg, false, &reporter).unwrap();

    let bundle_dir = project.path().join(&bundle);
    assert!(bundle_dir.join("app.ico").is_file());
    // Windows reads an external manifest named after the executable.
    assert!(
        bundle_dir
            .join(format!("{}.manifest", desktop_exe_name(pkg)))
            .is_file()
    );
}

#[test]
fn packaging_keeps_a_dashed_package_name() {
    let pkg = "my-cool-app";
    let project = desktop_project(pkg);
    let reporter = SpyReporter::new();

    let bundle = package_desktop_in(project.path(), Targets::Linux, pkg, false, &reporter).unwrap();

    assert!(
        project
            .path()
            .join(&bundle)
            .join(desktop_exe_name(pkg))
            .is_file()
    );
}

#[test]
fn packaging_removes_a_stale_bundle_first() {
    let pkg = "my_app";
    let project = desktop_project(pkg);
    let bundle = project
        .path()
        .join(desktop_bundle_path(Targets::Linux, pkg, false));
    std::fs::create_dir_all(&bundle).unwrap();
    std::fs::write(bundle.join("stale.txt"), b"old").unwrap();
    let reporter = SpyReporter::new();

    package_desktop_in(project.path(), Targets::Linux, pkg, false, &reporter).unwrap();

    assert!(!bundle.join("stale.txt").exists());
    assert!(bundle.join(desktop_exe_name(pkg)).is_file());
}

#[test]
fn packaging_without_a_compiled_binary_names_the_missing_executable() {
    let project = tempfile::tempdir().unwrap();
    let reporter = SpyReporter::new();

    // This name is never staged by `desktop_project`, so the resolved
    // executable is guaranteed absent.
    let err = package_desktop_in(
        project.path(),
        Targets::Linux,
        "my_app_missing",
        false,
        &reporter,
    )
    .expect_err("no compiled binary");

    assert!(err.to_string().contains("copying executable"), "{err}");
}

#[test]
fn packaging_survives_a_project_without_scaffold_files() {
    let pkg = "my_app";
    let project = desktop_project(pkg);
    let reporter = SpyReporter::new();

    let bundle = package_desktop_in(project.path(), Targets::Linux, pkg, false, &reporter).unwrap();

    assert!(
        project
            .path()
            .join(&bundle)
            .join(desktop_exe_name(pkg))
            .is_file()
    );
    assert!(
        reporter
            .notes
            .lock()
            .unwrap()
            .iter()
            .any(|n| n.contains("warning:"))
    );
}

#[test]
fn a_desktop_build_for_another_os_is_refused() {
    let reporter = SpyReporter::new();
    for target in [Targets::Windows, Targets::Linux] {
        if Targets::host_desktop() == Some(target) {
            continue;
        }
        let err =
            build_desktop(target, "my_app", false, &reporter).expect_err("cross-OS desktop build");
        let message = err.to_string();
        assert!(message.contains(&target.to_string()), "{message}");
        assert!(message.contains(std::env::consts::OS), "{message}");
        assert!(package_desktop(target, "my_app", false, &reporter).is_err());
    }
    // Nothing was run and nothing was written for a refused target.
    assert!(reporter.steps.lock().unwrap().is_empty());
}

#[test]
fn the_desktop_build_asks_cargo_for_the_application_binary() {
    let Some(host) = Targets::host_desktop() else {
        return;
    };
    let reporter = SpyReporter::new();
    build_desktop(host, "my-cool-app", true, &reporter).unwrap();

    let steps = reporter.steps.lock().unwrap();
    let (step, args) = steps.first().expect("a cargo step");
    assert_eq!(step.kind, StepKind::Cargo);
    assert_eq!(
        args,
        &vec![
            "build".to_string(),
            "--bin".to_string(),
            "my-cool-app".to_string(),
            "--release".to_string(),
        ]
    );
}

#[test]
fn retargeting_a_desktop_entry_points_at_the_bundled_binary() {
    let entry = retarget_desktop_entry(
        "[Desktop Entry]\nName=Demo\nExec=aimer_app\nIcon=aimer\nType=Application\n",
        "my_app",
    );

    assert_eq!(
        entry,
        "[Desktop Entry]\nName=Demo\nExec=my_app\nIcon=app\nType=Application\n"
    );
}

#[test]
fn device_plan_targets_the_iphoneos_sdk() {
    let plan = IosPlan::resolve(false);
    assert!(!plan.simulator);
    assert_eq!(plan.rust_target, "aarch64-apple-ios");
    assert_eq!(plan.sdk, "iphoneos");
}

#[test]
fn simulator_plan_follows_the_host_architecture() {
    let plan = IosPlan::resolve(true);
    assert!(plan.simulator);
    assert_eq!(plan.sdk, "iphonesimulator");
    let expected = if plan.arch == "x86_64" {
        "x86_64-apple-ios"
    } else {
        "aarch64-apple-ios-sim"
    };
    assert_eq!(plan.rust_target, expected);
}

#[test]
fn ios_app_path_follows_the_sdk_and_configuration() {
    assert_eq!(
        IosPlan::resolve(false).app_path("my_app", false),
        "builds/ios/build/Debug-iphoneos/my_app.app"
    );
    assert_eq!(
        IosPlan::resolve(true).app_path("my_app", true),
        "builds/ios/build/Release-iphonesimulator/my_app.app"
    );
}

#[test]
fn android_plan_maps_every_known_abi() {
    for (abi, rust_target, jni_dir) in [
        ("x86_64", "x86_64-linux-android", "x86_64"),
        ("armeabi-v7a", "armv7-linux-androideabi", "armeabi-v7a"),
        ("x86", "i686-linux-android", "x86"),
        ("arm64-v8a", "aarch64-linux-android", "arm64-v8a"),
    ] {
        let plan = AndroidPlan::for_abi(abi);
        assert_eq!(plan.rust_target, rust_target, "{abi}");
        assert_eq!(plan.jni_dir, jni_dir, "{abi}");
    }
}

#[test]
fn android_plan_falls_back_to_arm64_for_an_unknown_abi() {
    assert_eq!(
        AndroidPlan::for_abi("mips"),
        AndroidPlan::for_abi(ANDROID_DEFAULT_ABI)
    );
}

#[test]
fn jni_libs_dir_follows_the_abi() {
    assert_eq!(
        AndroidPlan::for_abi("x86_64").jni_libs_dir(),
        "builds/android/app/src/main/jniLibs/x86_64"
    );
}

#[test]
fn xcode_command_carries_the_configuration_and_no_sdk_on_macos() {
    let cmd = xcode_command(
        "builds/macos",
        "my_app",
        "My App",
        "Release",
        None,
        "arm64",
        None,
    );
    let args = args_of(&cmd);
    assert!(args.contains(&"-configuration".to_string()));
    assert!(args.contains(&"Release".to_string()));
    assert!(!args.contains(&"-sdk".to_string()));
    assert!(args.contains(&"my_app.xcodeproj".to_string()));
}

#[test]
fn xcode_command_names_the_product_after_the_application() {
    let cmd = xcode_command(
        "builds/macos",
        "my_app",
        "My App",
        "Debug",
        None,
        "arm64",
        None,
    );
    let args = args_of(&cmd);
    // The project and target keep the Cargo package; only the product renames.
    assert!(args.contains(&"my_app.xcodeproj".to_string()));
    assert!(args.contains(&"my_app".to_string()));
    assert!(args.contains(&"PRODUCT_NAME=My App".to_string()));
}

#[test]
fn macos_executable_is_named_after_the_application() {
    assert_eq!(
        macos_executable_path("builds/macos/build/Debug/My App.app", "My App"),
        "builds/macos/build/Debug/My App.app/Contents/MacOS/My App"
    );
}

#[test]
fn xcode_command_force_loads_the_rust_archive_for_hot_reload() {
    let cmd = xcode_command(
        "builds/macos",
        "my_app",
        "my_app",
        "Debug",
        None,
        "arm64",
        Some("$(SRCROOT)/Libraries/libmy_app.a"),
    );
    let args = args_of(&cmd);
    assert!(args.contains(
        &"OTHER_LDFLAGS=$(inherited) -force_load $(SRCROOT)/Libraries/libmy_app.a".to_string()
    ));
}

#[test]
fn xcode_command_carries_the_sdk_on_ios() {
    let cmd = xcode_command(
        "builds/ios",
        "my_app",
        "my_app",
        "Debug",
        Some("iphonesimulator"),
        "arm64",
        None,
    );
    let args = args_of(&cmd);
    assert!(args.contains(&"-sdk".to_string()));
    assert!(args.contains(&"iphonesimulator".to_string()));
}

#[test]
fn gradle_command_uses_the_task_for_the_mode() {
    let dir = Path::new("builds/android");
    assert_eq!(
        args_of(&gradle_command(dir, false)),
        vec!["assembleDebug".to_string()]
    );
    assert_eq!(
        args_of(&gradle_command(dir, true)),
        vec!["assembleRelease".to_string()]
    );
}

#[test]
fn apk_candidates_cover_the_unsigned_release_name() {
    assert_eq!(apk_candidates(false).len(), 1);
    let release = apk_candidates(true);
    assert_eq!(release.len(), 2);
    assert!(release[1].ends_with("app-release-unsigned.apk"));
}

#[test]
fn apk_path_falls_back_to_the_output_directory() {
    // Nothing is built in the test working directory, so the probe misses.
    assert_eq!(
        apk_path(false),
        "builds/android/app/build/outputs/apk/debug/"
    );
}

#[test]
fn build_macos_asks_cargo_for_the_requested_profile() {
    for release in [false, true] {
        let reporter = SpyReporter::new();
        build_macos(release, &reporter).unwrap();
        let steps = reporter.steps.lock().unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].0.kind, StepKind::Cargo);
        assert_eq!(
            steps[0].1.contains(&"--release".to_string()),
            release,
            "release = {release}"
        );
    }
}

#[test]
fn build_ios_asks_cargo_for_the_requested_profile() {
    let plan = IosPlan::resolve(true);
    for release in [false, true] {
        let reporter = SpyReporter::new();
        build_ios(&plan, release, &reporter).unwrap();
        let steps = reporter.steps.lock().unwrap();
        assert!(steps[0].1.contains(&plan.rust_target.to_string()));
        assert_eq!(steps[0].1.contains(&"--release".to_string()), release);
    }
}

#[test]
fn build_android_targets_the_abi_of_the_plan() {
    let plan = AndroidPlan::for_abi("x86_64");
    let reporter = SpyReporter::new();
    build_android(&plan, true, &reporter).unwrap();
    let steps = reporter.steps.lock().unwrap();
    assert_eq!(
        steps[0].1,
        vec!["ndk", "-t", "x86_64", "build", "--lib", "--release"]
    );
}

#[test]
fn execute_rejects_unknown_platform() {
    let err = execute("playstation".to_string(), false).unwrap_err();
    assert!(err.to_string().contains("unknown target"));
}

#[test]
fn execute_rejects_terminated_platform() {
    let err = execute("terminated".to_string(), false).unwrap_err();
    assert!(err.to_string().contains("not an assemblable platform"));
}
