use super::{
    RustAnalyzerBridge, flycheck_result_json, normalize_result, position_from_source, simple_glob,
    source_offset, unified_diff, workspace_edit_files,
};
use serde_json::json;
use std::fs;

#[test]
fn position_conversion_uses_one_based_utf16_columns() {
    assert_eq!(
        position_from_source("a😀b\n", 1, 4).unwrap(),
        json!({"line": 0, "character": 3})
    );
    assert!(position_from_source("a😀b\n", 1, 3).is_err());
}

#[test]
fn edit_offsets_handle_utf16_and_crlf_lines() {
    let source = "a😀b\r\nnext\r\n";
    assert_eq!(
        source_offset(source, &json!({"line": 0, "character": 3})).unwrap(),
        5
    );
    assert_eq!(
        source_offset(source, &json!({"line": 1, "character": 0})).unwrap(),
        8
    );
}

#[test]
fn project_globs_support_recursive_directories() {
    assert!(simple_glob("src/**/*.rs", "src/main.rs"));
    assert!(simple_glob("src/**/*.rs", "src/core/lib.rs"));
    assert!(!simple_glob("src/*.rs", "src/core/lib.rs"));
}

#[test]
fn diff_contains_removed_and_added_lines() {
    let diff = unified_diff(b"fn old() {}\n", b"fn new() {}\n");
    assert!(diff.contains("-fn old() {}"));
    assert!(diff.contains("+fn new() {}"));
}

#[test]
fn normalized_lsp_symbols_are_compact_and_serialize_stably() {
    let temporary = tempfile::tempdir().unwrap();
    let input = json!({
        "deprecated": false,
        "tags": [],
        "kind": 12,
        "name": "read_config",
        "range": {
            "start": { "line": 4, "character": 0 },
            "end": { "line": 6, "character": 1 }
        },
        "selectionRange": {
            "start": { "line": 5, "character": 7 },
            "end": { "line": 5, "character": 18 }
        }
    });
    let reordered_input: serde_json::Value = serde_json::from_str(
        r#"{
                "selectionRange":{"end":{"character":18,"line":5},"start":{"character":7,"line":5}},
                "range":{"end":{"character":1,"line":6},"start":{"character":0,"line":4}},
                "name":"read_config",
                "kind":12,
                "tags":[],
                "deprecated":false
            }"#,
    )
    .unwrap();

    let normalized = normalize_result(temporary.path(), &input);
    let repeated = normalize_result(temporary.path(), &reordered_input);
    let serialized = serde_json::to_string(&normalized).unwrap();
    let repeated_serialized = serde_json::to_string(&repeated).unwrap();

    assert_eq!(normalized, repeated);
    assert_eq!(serialized, repeated_serialized);
    assert_eq!(
        serialized,
        r#"{"kind":"function","name":"read_config","range":{"end":{"column":2,"line":7},"start":{"column":1,"line":5}},"selection_range":{"end":{"column":19,"line":6},"start":{"column":8,"line":6}}}"#
    );
}

#[test]
fn nested_lsp_result_truncation_is_reported() {
    let temporary = tempfile::tempdir().unwrap();
    let input = json!({
        "children": (0..=200)
            .map(|index| json!({ "name": format!("child_{index}") }))
            .collect::<Vec<_>>()
    });

    let normalized = normalize_result(temporary.path(), &input);

    assert_eq!(normalized["children"].as_array().unwrap().len(), 200);
    assert_eq!(normalized["children_truncated"], true);
}

#[test]
fn clippy_result_reports_diagnostics_without_claiming_a_cargo_exit_code() {
    let result = flycheck_result_json(json!({
        "diagnostics": [
            { "severity": "error", "source": "rustc", "message": "mismatched types" },
            { "severity": "warning", "source": "rustc", "message": "unused variable" }
        ],
        "truncated": false
    }));

    assert_eq!(result["operation"], "cargo_clippy");
    assert_eq!(result["command"], "clippy");
    assert_eq!(result["warnings_denied"], true);
    assert_eq!(result["status"], "completed_with_errors");
    assert_eq!(result["cargo_exit_status"], "not_reported_by_rust_analyzer");
    assert_eq!(result["diagnostic_summary"]["error_count"], 1);
    assert_eq!(result["diagnostic_summary"]["warning_count"], 1);
    assert_eq!(result["diagnostics"].as_array().unwrap().len(), 2);

    let no_errors = flycheck_result_json(json!({
        "diagnostics": [],
        "truncated": false
    }));
    assert_eq!(no_errors["status"], "completed");
    assert_eq!(no_errors["diagnostic_summary"]["error_count"], 0);
}

#[tokio::test]
async fn workspace_edit_preview_is_bounded_to_project_files() {
    let temporary = tempfile::tempdir().unwrap();
    let file = temporary.path().join("lib.rs");
    fs::write(&file, "fn old() {}\n").unwrap();
    let uri = reqwest::Url::from_file_path(&file).unwrap().to_string();
    let edit = json!({
        "changes": { uri: [{
            "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
            "newText": "new"
        }] }
    });
    let preview = workspace_edit_files(temporary.path(), &edit).await.unwrap();
    assert_eq!(preview.len(), 1);
    assert_eq!(preview[0].before.as_deref(), Some(&b"fn old() {}\n"[..]));
    assert_eq!(preview[0].after.as_deref(), Some(&b"fn new() {}\n"[..]));
    assert_eq!(fs::read(&file).unwrap(), b"fn old() {}\n");
}

#[tokio::test]
async fn failed_manifest_validation_prevents_apply_and_keeps_file_unchanged() {
    let temporary = tempfile::tempdir().unwrap();
    let manifest = temporary.path().join("Cargo.toml");
    let before = b"[package]\nname = \"sample\"\nversion = \"0.1.0\"\n";
    let after = b"[package]\nname = \"sample\"\nversion = \"invalid\"\n";
    fs::write(&manifest, before).unwrap();
    let bridge = RustAnalyzerBridge::new(
        temporary.path().to_path_buf(),
        aimer_hot_reload::config::RustAnalyzerConfig::default(),
    );
    let preview = bridge
        .prepare_manifest_content_preview(
            std::path::Path::new("Cargo.toml"),
            before,
            after,
            json!({ "status": "failed", "message": "candidate manifest is invalid" }),
            false,
        )
        .await
        .unwrap();
    assert_eq!(preview["applicable"], false);
    let preview_id = preview["preview_id"].as_str().unwrap();
    assert!(bridge.apply_preview(preview_id).await.is_err());
    assert_eq!(fs::read(&manifest).unwrap(), before);
    assert!(bridge.apply_preview(preview_id).await.is_err());
}
