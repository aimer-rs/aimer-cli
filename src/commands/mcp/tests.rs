use super::*;

#[test]
fn log_payload_omits_unspecified_fields() {
    let payload = logs_payload(LogsArguments {
        stream: Some("app".to_string()),
        ..LogsArguments::default()
    });
    assert_eq!(payload["stream"], "app");
    assert!(payload.get("cursor").is_none());
}

#[test]
fn log_payload_treats_all_as_the_default_stream() {
    let payload = logs_payload(LogsArguments {
        stream: Some("all".to_string()),
        limit: Some(5),
        ..LogsArguments::default()
    });

    assert!(payload.get("stream").is_none());
    assert_eq!(payload["limit"], 5);
}

#[test]
fn global_mcp_discovery_uses_the_nearest_aimer_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let nested = project.join("src/deep");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(project.join("Aimer.toml"), "[package]\nname = \"demo\"\n").unwrap();

    assert_eq!(
        discover_project_root(&nested).unwrap(),
        project.canonicalize().unwrap()
    );
}

#[test]
fn global_mcp_discovery_uses_a_cargo_manifest_for_rust_projects() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("rust-project");
    let nested = project.join("src/deep");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(project.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();

    assert_eq!(
        discover_project_root(&nested).unwrap(),
        project.canonicalize().unwrap()
    );
}

#[test]
fn global_mcp_discovery_rejects_directories_without_project_manifest() {
    let temp = tempfile::tempdir().unwrap();

    let error = discover_project_root(temp.path()).unwrap_err().to_string();

    assert!(error.contains("no Aimer.toml or Cargo.toml found"));
}

#[test]
fn encode_returns_structured_content_and_actionable_check_errors() {
    let success = encode(
        "search_symbols",
        Ok(json!({ "results": [], "truncated": false })),
    );
    assert_eq!(
        success.structured_content,
        Some(json!({ "results": [], "truncated": false }))
    );
    assert_eq!(success.is_error, Some(false));

    let error = encode(
        "check",
        Err(anyhow!(
            "rust-analyzer did not start flycheck within 30 seconds"
        )),
    );
    assert_eq!(error.is_error, Some(true));
    assert_eq!(
        error.structured_content,
        Some(json!({
            "error": {
                "code": "check_start_timeout",
                "operation": "check",
                "phase": "start",
                "message": "rust-analyzer did not start flycheck within 30 seconds"
            }
        }))
    );
}

#[test]
fn symbol_search_order_is_repeatable_and_keeps_exact_matches_first() {
    let symbols = vec![
        json!({
            "name": "WidgetBuilder",
            "kind": 12,
            "location": { "uri": "file:///workspace/src/b.rs", "range": { "start": { "line": 4, "character": 0 } } }
        }),
        json!({
            "name": "Widget",
            "kind": 23,
            "location": { "uri": "file:///workspace/src/z.rs", "range": { "start": { "line": 8, "character": 0 } } }
        }),
        json!({
            "name": "WidgetAdapter",
            "kind": 5,
            "location": { "uri": "file:///workspace/src/c.rs", "range": { "start": { "line": 2, "character": 0 } } }
        }),
        json!({
            "name": "Widget",
            "kind": 23,
            "location": { "uri": "file:///workspace/src/a.rs", "range": { "start": { "line": 3, "character": 0 } } }
        }),
    ];
    let mut forward = symbols.clone();
    let mut reverse = symbols.into_iter().rev().collect::<Vec<_>>();

    sort_rust_symbols(&mut forward, "Widget");
    sort_rust_symbols(&mut reverse, "Widget");

    assert_eq!(forward, reverse);
    assert!(rust_symbol_is_exact_match(&forward[0], "Widget"));
    assert!(rust_symbol_is_exact_match(&forward[1], "Widget"));
    assert_eq!(forward[0]["location"]["uri"], "file:///workspace/src/a.rs");
    assert!(!rust_symbol_is_exact_match(&forward[2], "Widget"));
    assert_eq!(
        serde_json::to_string(&forward).unwrap(),
        serde_json::to_string(&reverse).unwrap()
    );
}
