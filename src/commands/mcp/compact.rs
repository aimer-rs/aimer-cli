use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Map, Value, json};

use super::trim::{MAX_LIST_ITEMS, clean_text};

/// Environment variable that disables output compaction when set to anything
/// other than an empty string, `0`, or `false`.
pub(crate) const FULL_OUTPUT_ENV: &str = "AIMER_MCP_FULL_OUTPUT";

static FULL_OUTPUT: AtomicBool = AtomicBool::new(false);

tokio::task_local! {
    /// Per-client full-output choice for the tool call being served; a shared
    /// workspace host serves clients with different preferences.
    pub(super) static FULL_OUTPUT_SCOPE: bool;
}

/// Force full, uncompacted tool output for the lifetime of the process.
pub(crate) fn set_full_output(enabled: bool) {
    FULL_OUTPUT.store(enabled, Ordering::Relaxed);
}

pub(super) fn full_output_enabled() -> bool {
    FULL_OUTPUT_SCOPE
        .try_with(|enabled| *enabled)
        .unwrap_or(false)
        || FULL_OUTPUT.load(Ordering::Relaxed)
        || env_enabled(std::env::var(FULL_OUTPUT_ENV).ok())
}

fn env_enabled(value: Option<String>) -> bool {
    value.is_some_and(|value| {
        let value = value.trim();
        !(value.is_empty() || value == "0" || value.eq_ignore_ascii_case("false"))
    })
}

/// Reduce a tool result to its essential information.
///
/// - Diagnostics with a `rendered` text collapse to `file:line:col` plus the
///   rendered text; the fields it already contains are dropped.
/// - Zero counts, `false` flags and zero durations carry no information and
///   are removed, as is the top-level `operation` (it is the tool name).
pub(super) fn compact(value: Value) -> Value {
    match value {
        Value::Object(mut map) => {
            map.remove("operation");
            Value::Object(compact_object(map))
        }
        other => compact_value(other),
    }
}

fn compact_value(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(compact_object(map)),
        Value::Array(items) => Value::Array(items.into_iter().map(compact_value).collect()),
        Value::String(text) => Value::String(clean_text(&text)),
        other => other,
    }
}

fn compact_object(map: Map<String, Value>) -> Map<String, Value> {
    let map = if is_rendered_diagnostic(&map) {
        collapse_diagnostic(map)
    } else {
        map
    };
    map.into_iter()
        .map(|(key, value)| {
            let value = if key == "stderr" {
                strip_warning_summaries(value)
            } else {
                value
            };
            (key, value)
        })
        .filter(|(key, value)| !is_noise(key, value))
        .map(|(key, value)| (key, compact_value(value)))
        .fold(Map::new(), |mut map, (key, value)| {
            match value {
                Value::Array(mut items) if items.len() > MAX_LIST_ITEMS => {
                    map.insert(
                        format!("{key}_omitted"),
                        json!(items.len() - MAX_LIST_ITEMS),
                    );
                    items.truncate(MAX_LIST_ITEMS);
                    map.insert(key, Value::Array(items));
                }
                value => {
                    map.insert(key, value);
                }
            }
            map
        })
}

/// Keep errors in `stderr`; Cargo's "generated N warnings" summaries are noise.
fn strip_warning_summaries(value: Value) -> Value {
    let Value::String(text) = value else {
        return value;
    };
    let kept = text
        .lines()
        .filter(|line| !(line.starts_with("warning: `") && line.contains(" generated ")))
        .collect::<Vec<_>>()
        .join("\n");
    Value::String(kept)
}

fn is_rendered_diagnostic(map: &Map<String, Value>) -> bool {
    map.get("rendered").is_some_and(Value::is_string) && map.contains_key("severity")
}

fn collapse_diagnostic(mut map: Map<String, Value>) -> Map<String, Value> {
    let location = location(&map);
    let rendered = map.remove("rendered");
    let mut collapsed = Map::new();
    if let Some(location) = location {
        collapsed.insert("file".to_owned(), Value::String(location));
    }
    if let Some(rendered) = rendered {
        collapsed.insert("rendered".to_owned(), rendered);
    }
    collapsed
}

fn location(map: &Map<String, Value>) -> Option<String> {
    let file = map.get("file")?.as_str()?;
    let mut location = file.to_owned();
    if let Some(line) = map.get("line").and_then(Value::as_u64) {
        location.push_str(&format!(":{line}"));
        if let Some(column) = map.get("column").and_then(Value::as_u64) {
            location.push_str(&format!(":{column}"));
        }
    }
    Some(location)
}

fn is_noise(key: &str, value: &Value) -> bool {
    match value {
        Value::Bool(false) => key.ends_with("_truncated"),
        Value::String(text) => text.is_empty() && (key == "stdout" || key == "stderr"),
        Value::Number(number) => {
            number.as_u64() == Some(0)
                && (key == "duration_ms"
                    || key == "warning_count"
                    || key.ends_with("_filtered")
                    || key.ends_with("_lines_filtered"))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn clippy_result_keeps_only_location_and_rendered_text() {
        let compacted = compact(json!({
            "operation": "cargo_clippy",
            "status": "failed",
            "exit_code": 101,
            "duration_ms": 0,
            "diagnostic_summary": { "error_count": 1, "warning_count": 0 },
            "diagnostics": [{
                "file": "src/lib.rs",
                "line": 212,
                "column": 23,
                "severity": "error",
                "source": "clippy",
                "code": "clippy::unnecessary_map_or",
                "message": "simplify",
                "rendered": "error: simplify\n"
            }],
            "diagnostics_truncated": false,
            "cargo_progress_lines_filtered": 0,
            "lockfile_changed": false
        }));

        assert_eq!(
            compacted,
            json!({
                "status": "failed",
                "exit_code": 101,
                "diagnostic_summary": { "error_count": 1 },
                "diagnostics": [{
                    "file": "src/lib.rs:212:23",
                    "rendered": "error: simplify"
                }],
                "lockfile_changed": false
            })
        );
    }

    #[test]
    fn nonzero_noise_fields_are_kept() {
        let compacted = compact(json!({
            "duration_ms": 977,
            "diagnostic_summary": { "warning_count": 2 },
            "stdout_truncated": true,
            "warnings_filtered": 3
        }));

        assert_eq!(
            compacted,
            json!({
                "duration_ms": 977,
                "diagnostic_summary": { "warning_count": 2 },
                "stdout_truncated": true,
                "warnings_filtered": 3
            })
        );
    }

    #[test]
    fn stderr_keeps_errors_and_drops_warning_summaries() {
        let compacted = compact(json!({
            "stdout": "",
            "stderr": "warning: `demo` (lib) generated 2 warnings\nerror: could not compile `demo`"
        }));
        assert_eq!(
            compacted,
            json!({ "stderr": "error: could not compile `demo`" })
        );

        let compacted = compact(json!({
            "stderr": "warning: `demo` (lib) generated 2 warnings"
        }));
        assert_eq!(compacted, json!({}));
    }

    #[test]
    fn nested_operation_fields_are_not_removed() {
        let compacted = compact(json!({ "items": [{ "operation": "keep" }] }));

        assert_eq!(compacted, json!({ "items": [{ "operation": "keep" }] }));
    }

    #[test]
    fn long_lists_are_capped_with_an_omitted_count() {
        let items: Vec<_> = (0..80).map(|index| json!(index)).collect();
        let compacted = compact(json!({ "results": items }));

        assert_eq!(compacted["results"].as_array().unwrap().len(), 50);
        assert_eq!(compacted["results_omitted"], json!(30));
    }

    #[test]
    fn colour_codes_are_stripped_from_text_fields() {
        let compacted = compact(json!({ "stderr": "\u{1b}[31merror\u{1b}[0m: bad" }));

        assert_eq!(compacted, json!({ "stderr": "error: bad" }));
    }

    #[test]
    fn full_output_environment_values() {
        assert!(!env_enabled(None));
        assert!(!env_enabled(Some(String::new())));
        assert!(!env_enabled(Some("0".to_owned())));
        assert!(!env_enabled(Some("False".to_owned())));
        assert!(env_enabled(Some("1".to_owned())));
    }
}
