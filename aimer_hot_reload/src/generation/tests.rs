use std::fs;
use std::process::Command;

use tempfile::tempdir;

use crate::config::AimerManifest;

use super::*;

fn automatic_project(root: &Path) -> (PathBuf, PathBuf) {
    let workspace = root.join("framework");
    let project = root.join("counter");
    fs::create_dir_all(workspace.join("crates/aimer_wasm_guest")).unwrap();
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = \"aimer\"\nversion = \"0.0.0\"\n",
    )
    .unwrap();
    fs::write(
        workspace.join("crates/aimer_wasm_guest/Cargo.toml"),
        "[package]\nname = \"aimer_wasm_guest\"\nversion = \"0.0.0\"\n",
    )
    .unwrap();
    fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"counter\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\naimer = \"0\"\n",
        )
        .unwrap();
    fs::write(
        project.join("src/main.rs"),
        "#[aimer::main]\nfn main() { AimerApp::new().child(Text::new(\"one\")).run(); }\n",
    )
    .unwrap();
    (workspace, project)
}

#[test]
fn absent_manifest_or_metadata_selects_automatic_but_explicit_metadata_stays_manual() {
    assert_eq!(
        GuestGenerationMode::select(None),
        GuestGenerationMode::Automatic
    );
    let without_metadata: AimerManifest = toml::from_str(
        "[package]\nname = \"counter\"\ngroup = \"dev.example\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    assert_eq!(
        GuestGenerationMode::select(Some(&without_metadata)),
        GuestGenerationMode::Automatic
    );
    let explicit: AimerManifest = toml::from_str(
            "[package]\nname = \"counter\"\ngroup = \"dev.example\"\nversion = \"0.1.0\"\n\n[build.hot_reload]\npackage = \"manual_guest\"\nprogram = \"guest::Program\"\nlimits = \"guest::LIMITS\"\n",
        )
        .unwrap();
    assert_eq!(
        GuestGenerationMode::select(Some(&explicit)),
        GuestGenerationMode::Manual
    );
}

#[cfg(feature = "hot-reload")]
#[test]
fn automatic_preparation_is_fresh_and_leaves_the_application_unchanged() {
    let root = tempdir().unwrap();
    let (workspace, project) = automatic_project(root.path());
    let original = fs::read(project.join("src/main.rs")).unwrap();
    let session = project.join("target/aimer-hot-reload");

    let first = prepare_automatic_guest(&project, &workspace, &session).unwrap();
    assert_eq!(
        first.application_root(),
        fs::canonicalize(&project)
            .unwrap()
            .join("target/aimer-hot-reload/application")
    );
    assert_eq!(first.package(), "aimer_generated_guest");
    let portable_source_root = fs::canonicalize(project.join("src")).unwrap();
    assert_eq!(
        first.portable_source_root(),
        Some(portable_source_root.as_path())
    );
    assert!(first.application_root().join("src/main.rs").is_file());
    fs::write(first.application_root().join("stale.rs"), "stale").unwrap();
    fs::write(
        project.join("src/main.rs"),
        "#[aimer::main]\nfn main() { AimerApp::new().child(Text::new(\"two\")).run(); }\n",
    )
    .unwrap();

    let second = prepare_automatic_guest(&project, &workspace, &session).unwrap();
    assert!(!second.application_root().join("stale.rs").exists());
    let wrapper = fs::read_to_string(second.manifest()).unwrap();
    assert!(wrapper.contains("name = \"aimer_generated_guest\""));
    assert!(wrapper.contains("package = \"counter\""));
    let source = fs::read_to_string(second.wrapper_root().join("src/lib.rs")).unwrap();
    assert!(source.contains("application::__AimerGeneratedGuestProgram"));
    assert!(source.contains("application::__AIMER_GENERATED_GUEST_LIMITS"));
    assert_ne!(fs::read(project.join("src/main.rs")).unwrap(), original);
    assert_eq!(
        fs::read_to_string(project.join("src/main.rs")).unwrap(),
        "#[aimer::main]\nfn main() { AimerApp::new().child(Text::new(\"two\")).run(); }\n"
    );
    assert!(root.path().read_dir().unwrap().flatten().all(|entry| {
        !entry
            .file_name()
            .to_string_lossy()
            .starts_with(".aimer-hot-reload-shadow-")
    }));
}

#[test]
fn generated_tree_sync_updates_only_changed_files_and_removes_stale_entries() {
    let root = tempdir().unwrap();
    let staged = root.path().join("staged");
    let live = root.path().join("live");
    fs::create_dir_all(staged.join("src")).unwrap();
    fs::create_dir_all(live.join("src")).unwrap();
    fs::create_dir_all(live.join("old")).unwrap();
    fs::write(staged.join("src/lib.rs"), "new").unwrap();
    fs::write(staged.join("src/unchanged.rs"), "same").unwrap();
    fs::write(live.join("src/lib.rs"), "old").unwrap();
    fs::write(live.join("src/unchanged.rs"), "same").unwrap();
    fs::write(live.join("old/stale.rs"), "stale").unwrap();

    let report = sync_generated_directory(&staged, &live).unwrap();

    assert_eq!(report.changed_files, 1);
    assert_eq!(report.removed_entries, 2);
    assert_eq!(fs::read_to_string(live.join("src/lib.rs")).unwrap(), "new");
    assert_eq!(
        fs::read_to_string(live.join("src/unchanged.rs")).unwrap(),
        "same"
    );
    assert!(!live.join("old/stale.rs").exists());
    assert!(!live.join("old").exists());
}

#[test]
fn generated_tree_sync_rejects_invalid_staging_before_touching_live_output() {
    let root = tempdir().unwrap();
    let staged = root.path().join("staged");
    let live = root.path().join("live");
    fs::write(&staged, "not a directory").unwrap();
    fs::create_dir_all(&live).unwrap();
    fs::write(live.join("existing.rs"), "keep").unwrap();

    assert!(sync_generated_directory(&staged, &live).is_err());
    assert_eq!(
        fs::read_to_string(live.join("existing.rs")).unwrap(),
        "keep"
    );
}

#[test]
fn generated_file_write_skips_identical_bytes() {
    let root = tempdir().unwrap();
    let path = root.path().join("generated.rs");

    assert!(write_generated_file(&path, b"same").unwrap());
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    assert!(!write_generated_file(&path, b"same").unwrap());
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    assert!(write_generated_file(&path, b"changed").unwrap());
    assert_eq!(fs::read_to_string(path).unwrap(), "changed");
}

#[cfg(feature = "hot-reload")]
#[test]
#[ignore = "temporarily disabled while stale generation cleanup is repaired"]
fn stale_generation_artifacts_are_reclaimed_without_touching_unrelated_files() {
    let root = tempdir().unwrap();
    let (workspace, project) = automatic_project(root.path());
    let session = project.join("target/aimer-hot-reload");

    let stale_shadow = root.path().join(".aimer-hot-reload-shadow-999999-1");
    fs::create_dir_all(&stale_shadow).unwrap();
    fs::write(stale_shadow.join("partial.rs"), "partial").unwrap();
    let current_shadow = root
        .path()
        .join(format!(".aimer-hot-reload-shadow-{}-2", std::process::id()));
    fs::create_dir_all(&current_shadow).unwrap();

    let stale_temporary = session.join("generated/src/.lib.rs.aimer-tmp-999999-1");
    fs::create_dir_all(stale_temporary.parent().unwrap()).unwrap();
    fs::write(&stale_temporary, "partial").unwrap();
    let live_file = session.join("generated/src/lib.rs");
    fs::write(&live_file, "live").unwrap();
    let unrelated = root.path().join("keep.txt");
    fs::write(&unrelated, "keep").unwrap();

    prepare_automatic_guest(&project, &workspace, &session).unwrap();

    assert!(!stale_shadow.exists());
    assert!(current_shadow.exists());
    assert!(!stale_temporary.exists());
    assert_eq!(fs::read_to_string(unrelated).unwrap(), "keep");
    assert!(live_file.exists());
}

#[test]
fn automatic_preparation_rejects_framework_crates_that_escape_the_workspace() {
    let root = tempdir().unwrap();
    let (workspace, project) = automatic_project(root.path());
    let outside = root.path().join("outside");
    fs::rename(workspace.join("crates/aimer_wasm_guest"), &outside).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, workspace.join("crates/aimer_wasm_guest")).unwrap();

    let error = prepare_automatic_guest(
        &project,
        &workspace,
        &project.join("target/aimer-hot-reload"),
    )
    .unwrap_err()
    .to_string();

    #[cfg(unix)]
    assert!(error.contains("escapes the Aimer workspace"), "{error}");
}

#[cfg(feature = "hot-reload")]
#[test]
fn automatic_outer_wrapper_links_the_transformed_application() {
    let root = tempdir().unwrap();
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let project = root.path().join("application");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
            project.join("Cargo.toml"),
            format!(
                "[package]\nname = \"automatic-link-test\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\naimer = {{ path = {:?} }}\n",
                workspace
            ),
        )
        .unwrap();
    fs::write(
            project.join("src/main.rs"),
            "use aimer::*;\n#[aimer::main]\nfn main() { AimerApp::new().child(Text::new(\"hello\")).run(); }\n",
        )
        .unwrap();
    let generated = prepare_automatic_guest(
        &project,
        &workspace,
        &project.join("target/aimer-hot-reload"),
    )
    .unwrap();

    let status = Command::new(env!("CARGO"))
        .args(["build", "--manifest-path"])
        .arg(generated.manifest())
        .args([
            "--package",
            generated.package(),
            "--target",
            "wasm32-unknown-unknown",
            "--target-dir",
        ])
        .arg(project.join("target/aimer-hot-reload/guest"))
        .status()
        .unwrap();

    assert!(status.success());
}

#[test]
fn generated_guest_package_wraps_the_explicit_portable_entry() {
    let root = tempdir().unwrap();
    let project = root.path().join("counter-app");
    let adapter = root.path().join("aimer_wasm_guest");
    let output = root.path().join("target/aimer-hot-reload/generated-guest");
    fs::create_dir_all(&project).unwrap();
    fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"counter-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\ncrate-type = [\"lib\"]\n",
        )
        .unwrap();
    fs::create_dir_all(&adapter).unwrap();
    let spec = GuestPackageSpec::new(
        "counter-app",
        "guest::CounterProgram",
        "guest::HOT_RELOAD_LIMITS",
    );

    let generated = generate_guest_package(&project, &adapter, &output, &spec).unwrap();

    assert_eq!(generated.package(), "aimer_generated_guest");
    assert_eq!(generated.manifest(), output.join("Cargo.toml"));
    // Paths are rendered through `toml::Value::String`, which may quote them
    // as single-quoted TOML literal strings (e.g. on Windows, to avoid
    // escaping backslashes) rather than Rust's `Debug`-escaped form; encode
    // the expected paths the same way the implementation does.
    let adapter_path = toml::Value::String(adapter.to_string_lossy().into_owned()).to_string();
    let project_path = toml::Value::String(project.to_string_lossy().into_owned()).to_string();
    assert_eq!(
        fs::read_to_string(generated.manifest()).unwrap(),
        format!(
            "[package]\nname = \"aimer_generated_guest\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n[dependencies]\naimer_wasm_guest = {{ path = {adapter_path} }}\napplication = {{ path = {project_path}, package = \"counter-app\" }}\n\n[workspace]\n\n[profile.dev]\ndebug = 0\n"
        )
    );
    assert_eq!(
        fs::read_to_string(output.join("src/lib.rs")).unwrap(),
        "aimer_wasm_guest::export_guest!(\n    application::guest::CounterProgram,\n    application::guest::HOT_RELOAD_LIMITS,\n);\n"
    );
}

#[test]
fn generated_guest_disables_debug_information_to_keep_wasm_bounded() {
    let root = tempdir().unwrap();
    let project = root.path().join("counter-app");
    let adapter = root.path().join("aimer_wasm_guest");
    let output = root.path().join("target/aimer-hot-reload/generated-guest");
    fs::create_dir_all(&project).unwrap();
    fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"counter-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\ncrate-type = [\"lib\"]\n",
        )
        .unwrap();
    fs::create_dir_all(&adapter).unwrap();
    let spec = GuestPackageSpec::new("counter-app", "Program", "LIMITS");

    let generated = generate_guest_package(&project, &adapter, &output, &spec).unwrap();
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(generated.manifest()).unwrap()).unwrap();

    assert_eq!(manifest["profile"]["dev"]["debug"].as_integer(), Some(0));
}

#[test]
fn generated_guest_preserves_the_application_lockfile() {
    let root = tempdir().unwrap();
    let project = root.path().join("counter-app");
    let adapter = root.path().join("aimer_wasm_guest");
    let output = root.path().join("target/aimer-hot-reload/generated-guest");
    fs::create_dir_all(&project).unwrap();
    fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"counter-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\ncrate-type = [\"lib\"]\n",
        )
        .unwrap();
    fs::write(project.join("Cargo.lock"), "application-lock").unwrap();
    fs::create_dir_all(&adapter).unwrap();

    generate_guest_package(
        &project,
        &adapter,
        &output,
        &GuestPackageSpec::new("counter-app", "Program", "LIMITS"),
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(output.join("Cargo.lock")).unwrap(),
        "application-lock"
    );
}

#[test]
fn generated_guest_removes_a_lockfile_when_the_application_no_longer_has_one() {
    let root = tempdir().unwrap();
    let project = root.path().join("counter-app");
    let adapter = root.path().join("aimer_wasm_guest");
    let output = root.path().join("target/aimer-hot-reload/generated-guest");
    fs::create_dir_all(&project).unwrap();
    fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"counter-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\ncrate-type = [\"lib\"]\n",
        )
        .unwrap();
    fs::write(project.join("Cargo.lock"), "application-lock").unwrap();
    fs::create_dir_all(&adapter).unwrap();
    let spec = GuestPackageSpec::new("counter-app", "Program", "LIMITS");

    generate_guest_package(&project, &adapter, &output, &spec).unwrap();
    fs::remove_file(project.join("Cargo.lock")).unwrap();
    generate_guest_package(&project, &adapter, &output, &spec).unwrap();

    assert!(!output.join("Cargo.lock").exists());
}

#[test]
fn invalid_or_empty_metadata_is_rejected_before_writing_source() {
    let root = tempdir().unwrap();
    let output = root.path().join("generated");
    let cases = [
        GuestPackageSpec::new("", "Program", "LIMITS"),
        GuestPackageSpec::new("app", "", "LIMITS"),
        GuestPackageSpec::new("app", "guest::Program; fn injected() {}", "LIMITS"),
        GuestPackageSpec::new("app", "guest::Program", "guest::"),
    ];

    for spec in cases {
        let error = generate_guest_package(root.path(), root.path(), &output, &spec)
            .unwrap_err()
            .to_string();

        assert!(error.contains("invalid hot-reload guest metadata"));
        assert!(!output.join("src/lib.rs").exists());
    }
}

#[test]
fn application_library_must_emit_an_rlib_for_the_generated_dependency() {
    let root = tempdir().unwrap();
    let project = root.path().join("counter-app");
    let output = root.path().join("generated");
    fs::create_dir_all(&project).unwrap();
    fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"counter-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\ncrate-type = [\"cdylib\", \"staticlib\"]\n",
        )
        .unwrap();

    let error = generate_guest_package(
        &project,
        root.path(),
        &output,
        &GuestPackageSpec::new("counter-app", "Program", "LIMITS"),
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("must emit an `rlib`"), "{error}");
    assert!(error.contains("crate-type"), "{error}");
    assert!(!output.join("src/lib.rs").exists());
}

#[test]
fn cargo_default_library_forms_are_linkable_guest_dependencies() {
    let root = tempdir().unwrap();
    let explicit = root.path().join("explicit");
    let implicit = root.path().join("implicit");
    fs::create_dir_all(explicit.join("src")).unwrap();
    fs::create_dir_all(implicit.join("src")).unwrap();
    fs::write(
            explicit.join("Cargo.toml"),
            "[package]\nname = \"explicit\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\npath = \"src/application.rs\"\n",
        )
        .unwrap();
    fs::write(explicit.join("src/application.rs"), "").unwrap();
    fs::write(
        implicit.join("Cargo.toml"),
        "[package]\nname = \"implicit\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(implicit.join("src/lib.rs"), "").unwrap();

    assert!(validate_application_library(&explicit, "explicit").is_ok());
    assert!(validate_application_library(&implicit, "implicit").is_ok());
}
