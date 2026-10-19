use super::{
    ManifestOperation, ManifestOperationAction, apply_manifest_operation, project_metadata_value,
    resolve_manifest,
};
use serde_json::json;
use std::fs;
use toml_edit::DocumentMut;

#[test]
fn generic_manifest_edits_preserve_comments_and_replace_values() {
    let mut document = "# keep this comment\n[dependencies]\nserde = \"1\"\n"
        .parse::<DocumentMut>()
        .unwrap();
    apply_manifest_operation(
        &mut document,
        &ManifestOperation {
            action: ManifestOperationAction::Set,
            path: vec!["dependencies".to_owned(), "serde".to_owned()],
            value: Some(json!({ "version": "2", "features": ["derive"] })),
        },
    )
    .unwrap();
    let source = document.to_string();
    assert!(source.contains("# keep this comment"));
    assert!(source.contains("serde"));
    let parsed = toml_edit::de::from_str::<serde_json::Value>(&source).unwrap();
    assert_eq!(parsed["dependencies"]["serde"]["version"], "2");
    assert_eq!(
        parsed["dependencies"]["serde"]["features"],
        json!(["derive"])
    );
}

#[test]
fn nested_edit_updates_inline_dependency_tables() {
    let mut document = "[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\n"
        .parse::<DocumentMut>()
        .unwrap();
    apply_manifest_operation(
        &mut document,
        &ManifestOperation {
            action: ManifestOperationAction::Set,
            path: vec![
                "dependencies".to_owned(),
                "serde".to_owned(),
                "features".to_owned(),
            ],
            value: Some(json!(["derive", "rc"])),
        },
    )
    .unwrap();
    let parsed = toml_edit::de::from_str::<serde_json::Value>(&document.to_string()).unwrap();
    assert_eq!(
        parsed["dependencies"]["serde"]["features"],
        json!(["derive", "rc"])
    );
}

#[test]
fn manifest_remove_requires_existing_path() {
    let mut document = "[features]\ndefault = []\n".parse::<DocumentMut>().unwrap();
    let error = apply_manifest_operation(
        &mut document,
        &ManifestOperation {
            action: ManifestOperationAction::Remove,
            path: vec!["features".to_owned(), "missing".to_owned()],
            value: None,
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("does not exist"));
}

#[test]
fn manifest_path_must_remain_inside_selected_project() {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(
        temporary.path().join("Cargo.toml"),
        "[package]\nname='x'\nversion='0.1.0'\n",
    )
    .unwrap();
    assert!(resolve_manifest(temporary.path(), Some("../Cargo.toml")).is_err());
}

#[test]
fn metadata_projection_bounds_package_and_dependency_graph_lists() {
    let packages = (0..205)
        .map(|index| {
            json!({
                "id": format!("pkg-{index}"),
                "name": format!("pkg-{index}"),
                "manifest_path": "/workspace/Cargo.toml",
                "targets": [], "features": {}, "dependencies": []
            })
        })
        .collect::<Vec<_>>();
    let nodes = (0..205)
        .map(|index| json!({ "id": format!("pkg-{index}") }))
        .collect::<Vec<_>>();
    let result = project_metadata_value(
        std::path::Path::new("/workspace"),
        &json!({
            "workspace_root": "/workspace",
            "workspace_members": [],
            "workspace_default_members": [],
            "packages": packages,
            "resolve": { "root": null, "nodes": nodes }
        }),
    );
    assert_eq!(result["packages"].as_array().unwrap().len(), 200);
    assert_eq!(result["packages_truncated"], true);
    assert_eq!(result["resolve"]["nodes"].as_array().unwrap().len(), 200);
    assert_eq!(result["resolve"]["nodes_truncated"], true);
}

#[test]
fn cargo_output_is_bounded_and_reports_truncation() {
    let output = vec![b'x'; super::MAX_TOOL_OUTPUT_BYTES + 10];
    let (text, truncated) = super::bounded_text(&output);
    assert_eq!(text.len(), super::MAX_TOOL_OUTPUT_BYTES);
    assert!(truncated);
}

#[test]
fn cargo_json_output_filters_rustc_warnings_and_keeps_useful_output() {
    use super::normalize_cargo_output;

    let warning = json!({
        "reason": "compiler-message",
        "message": {
            "level": "warning",
            "message": "unused item",
            "spans": [],
            "rendered": "warning: unused item\n"
        }
    });
    let error = json!({
        "reason": "compiler-message",
        "message": {
            "level": "error",
            "message": "type mismatch",
            "spans": [],
            "rendered": "error: type mismatch\n"
        }
    });
    let artifact = json!({ "reason": "compiler-artifact" });
    let finished = json!({ "reason": "build-finished", "success": false });
    let build_script = json!({
        "reason": "build-script-executed",
        "out": "cargo:rerun-if-changed=build.rs\ncargo:warning=script warning\n"
    });
    let stdout = format!(
        "test output\n{}\n{}\n{}\n{}\n{}\n{{\"reason\":\n{{\"application\":\"output\"}}\n",
        warning, error, artifact, build_script, finished,
    );

    let (stdout, stderr, warnings_filtered) =
        normalize_cargo_output(stdout.as_bytes(), b"cargo status");

    assert_eq!(warnings_filtered, 1);
    let stdout = String::from_utf8(stdout).unwrap();
    assert!(stdout.contains("test output\n"));
    assert!(stdout.contains("{\"reason\":\n"));
    assert!(stdout.contains("{\"application\":\"output\"}"));
    assert!(!stdout.contains("compiler-artifact"));
    assert!(!stdout.contains("build-finished"));
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(stderr.contains("cargo status\nerror: type mismatch\n"));
    assert!(stderr.contains("warning: script warning\n"));
    assert!(!stderr.contains("unused item"));
}

#[test]
fn cargo_arguments_reject_ambiguous_or_option_like_values() {
    use super::{CargoRunArguments, validate_cargo_arguments};

    let error = validate_cargo_arguments(
        false,
        &CargoRunArguments {
            test_filter: Some("smoke".to_owned()),
            ..CargoRunArguments::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("only valid"));

    let error = validate_cargo_arguments(
        true,
        &CargoRunArguments {
            features: vec!["serde".to_owned()],
            all_features: true,
            ..CargoRunArguments::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("cannot be used together"));
}

#[tokio::test]
async fn cargo_tools_return_real_check_and_test_status_and_lockfile_change() {
    use super::{CargoRunArguments, run_cargo};

    let temporary = tempfile::tempdir().unwrap();
    fs::create_dir(temporary.path().join("src")).unwrap();
    fs::write(
        temporary.path().join("Cargo.toml"),
        "[package]\nname = \"mcp-cargo-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        temporary.path().join("src/lib.rs"),
        "fn unused_warning_fixture() {}\n#[test]\nfn smoke() {}\npub fn value() -> u32 { 42 }\n",
    )
    .unwrap();

    let checked = run_cargo(
        temporary.path(),
        false,
        false,
        CargoRunArguments::default(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(checked["status"], "passed");
    assert_eq!(checked["exit_code"], 0);
    assert_eq!(checked["lockfile_changed"], true);
    assert!(checked["warnings_filtered"].as_u64().unwrap() >= 1);
    assert!(!checked["stdout"].as_str().unwrap().contains("warning:"));
    assert!(
        !checked["stderr"]
            .as_str()
            .unwrap()
            .contains("warning: unused")
    );

    fs::write(
        temporary.path().join("src/lib.rs"),
        "pub fn value() -> u32 { \"not a number\" }\n",
    )
    .unwrap();
    let failed = run_cargo(
        temporary.path(),
        false,
        false,
        CargoRunArguments::default(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["exit_code"], 101);
    assert!(
        failed["stderr"]
            .as_str()
            .unwrap()
            .contains("mismatched types")
    );

    fs::write(
        temporary.path().join("src/lib.rs"),
        "fn unused_warning_fixture() {}\n#[test]\nfn smoke() {}\npub fn value() -> u32 { 42 }\n",
    )
    .unwrap();
    let tested = run_cargo(
        temporary.path(),
        false,
        true,
        CargoRunArguments {
            test_filter: Some("smoke".to_owned()),
            ..CargoRunArguments::default()
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(tested["status"], "passed");
    assert_eq!(tested["exit_code"], 0);
    assert!(tested["stdout"].as_str().unwrap().contains("smoke"));
    assert!(tested["warnings_filtered"].as_u64().unwrap() >= 1);
    assert!(!tested["stdout"].as_str().unwrap().contains("warning:"));
    assert!(
        !tested["stderr"]
            .as_str()
            .unwrap()
            .contains("warning: unused")
    );
}

#[tokio::test]
async fn manifest_preview_validates_in_a_staged_workspace_without_touching_project_files() {
    use super::{
        ManifestEditArguments, ManifestOperation, ManifestOperationAction, prepare_manifest_edit,
    };

    let temporary = tempfile::tempdir().unwrap();
    fs::create_dir(temporary.path().join("src")).unwrap();
    let manifest = temporary.path().join("Cargo.toml");
    let original =
        "[package]\nname = \"mcp-manifest-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
    fs::write(&manifest, original).unwrap();
    fs::write(temporary.path().join("src/lib.rs"), "pub fn value() {}\n").unwrap();

    let candidate = prepare_manifest_edit(
        temporary.path(),
        false,
        ManifestEditArguments {
            manifest_path: "Cargo.toml".to_owned(),
            operations: vec![ManifestOperation {
                action: ManifestOperationAction::Set,
                path: vec!["package".to_owned(), "version".to_owned()],
                value: Some(json!("0.2.0")),
            }],
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(candidate.validation["status"], "passed");
    assert!(candidate.applicable);
    assert_ne!(candidate.before, candidate.after);
    assert_eq!(fs::read_to_string(&manifest).unwrap(), original);
    assert!(!temporary.path().join("Cargo.lock").exists());
}

#[tokio::test]
async fn metadata_read_does_not_create_a_missing_lockfile() {
    use super::read_project_metadata;

    let temporary = tempfile::tempdir().unwrap();
    fs::create_dir(temporary.path().join("src")).unwrap();
    fs::write(
        temporary.path().join("Cargo.toml"),
        "[package]\nname = \"mcp-metadata-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let metadata = read_project_metadata(temporary.path(), false)
        .await
        .unwrap();
    assert_eq!(metadata["status"], "unavailable");
    assert!(!temporary.path().join("Cargo.lock").exists());
}

#[tokio::test]
async fn manifest_reader_returns_structured_toml() {
    use super::{ManifestReadArguments, read_manifest};

    let temporary = tempfile::tempdir().unwrap();
    fs::write(
            temporary.path().join("Cargo.toml"),
            "[package]\nname = \"mcp-manifest-reader\"\nversion = \"0.1.0\"\n[features]\ndefault = []\n",
        )
        .unwrap();
    let result = read_manifest(
        temporary.path(),
        ManifestReadArguments {
            manifest_path: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(result["manifest_path"], "Cargo.toml");
    assert_eq!(result["manifest"]["package"]["name"], "mcp-manifest-reader");
    assert_eq!(result["manifest"]["features"]["default"], json!([]));
}

#[tokio::test]
async fn cargo_manifest_validation_error_makes_preview_non_applicable() {
    use super::{
        ManifestEditArguments, ManifestOperation, ManifestOperationAction, prepare_manifest_edit,
    };

    let temporary = tempfile::tempdir().unwrap();
    fs::create_dir(temporary.path().join("src")).unwrap();
    fs::write(
        temporary.path().join("Cargo.toml"),
        "[package]\nname = \"mcp-invalid-manifest\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(temporary.path().join("src/lib.rs"), "pub fn value() {}\n").unwrap();
    let candidate = prepare_manifest_edit(
        temporary.path(),
        false,
        ManifestEditArguments {
            manifest_path: "Cargo.toml".to_owned(),
            operations: vec![ManifestOperation {
                action: ManifestOperationAction::Set,
                path: vec!["package".to_owned(), "version".to_owned()],
                value: Some(json!(7)),
            }],
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(candidate.validation["status"], "failed");
    assert!(!candidate.applicable);
    assert!(!candidate.validation["message"].as_str().unwrap().is_empty());
    assert!(!temporary.path().join("Cargo.lock").exists());
}

#[tokio::test]
async fn project_metadata_reads_locked_workspace_without_changing_lockfile() {
    use super::read_project_metadata;

    let temporary = tempfile::tempdir().unwrap();
    fs::create_dir(temporary.path().join("src")).unwrap();
    fs::write(
        temporary.path().join("Cargo.toml"),
        "[package]\nname = \"mcp-metadata-ready\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(temporary.path().join("src/lib.rs"), "pub fn value() {}\n").unwrap();
    fs::write(
        temporary.path().join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"mcp-metadata-ready\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let original_lock = fs::read(temporary.path().join("Cargo.lock")).unwrap();
    let metadata = read_project_metadata(temporary.path(), false)
        .await
        .unwrap();
    assert_eq!(metadata["status"], "ready");
    assert_eq!(metadata["packages"][0]["name"], "mcp-metadata-ready");
    assert_eq!(
        fs::read(temporary.path().join("Cargo.lock")).unwrap(),
        original_lock
    );
}
