use super::macro_references::diagnostic_severity_name;
use super::*;

pub(super) fn actions_len_exceeds(value: &Value, limit: usize) -> bool {
    value.as_array().is_some_and(|items| items.len() > limit)
}

pub(super) fn path_to_uri(path: &Path) -> anyhow::Result<String> {
    reqwest::Url::from_file_path(path)
        .map(|url| url.to_string())
        .map_err(|_| anyhow!("cannot convert '{}' to a file URI", path.display()))
}

pub(super) fn uri_to_relative(root: &Path, uri: &str) -> Option<String> {
    let path = reqwest::Url::parse(uri).ok()?.to_file_path().ok()?;
    let relative = path.strip_prefix(root).ok()?;
    Some(relative.to_string_lossy().replace('\\', "/"))
}

pub(super) fn normalize_range(value: Option<&Value>) -> Value {
    let Some(value) = value else {
        return Value::Null;
    };
    let normalize_position = |position: Option<&Value>| -> Value {
        let Some(position) = position else {
            return Value::Null;
        };
        json!({
            "line": position.get("line").and_then(Value::as_u64).unwrap_or(0) + 1,
            "column": position.get("character").and_then(Value::as_u64).unwrap_or(0) + 1
        })
    };
    json!({ "start": normalize_position(value.get("start")), "end": normalize_position(value.get("end")) })
}

pub(super) fn normalize_lsp_value(root: &Path, value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(MAX_RESULTS)
                .map(|item| normalize_lsp_value(root, item))
                .collect(),
        ),
        Value::Object(object) => {
            let mut normalized = Map::new();
            for (key, value) in object {
                if value.is_null()
                    || (key == "deprecated" && value == &Value::Bool(false))
                    || (key == "tags" && value.as_array().is_some_and(Vec::is_empty))
                {
                    continue;
                }
                let mapped_key = match key.as_str() {
                    "uri" => "file",
                    "targetUri" => "target_file",
                    "range" => "range",
                    "targetRange" => "target_range",
                    "targetSelectionRange" => "target_selection_range",
                    "selectionRange" => "selection_range",
                    other => other,
                };
                let mapped_value = match key.as_str() {
                    "uri" | "targetUri" => value
                        .as_str()
                        .and_then(|uri| uri_to_relative(root, uri))
                        .or_else(|| {
                            value.as_str().and_then(|uri| {
                                reqwest::Url::parse(uri)
                                    .ok()?
                                    .to_file_path()
                                    .ok()
                                    .map(|path| path.to_string_lossy().into_owned())
                            })
                        })
                        .map(|value| Value::String(value.to_owned()))
                        .unwrap_or_else(|| value.clone()),
                    "kind" if value.is_number() => value
                        .as_u64()
                        .map(symbol_kind_name)
                        .map(|value| Value::String(value.to_owned()))
                        .unwrap_or_else(|| value.clone()),
                    "severity" if value.is_number() => value
                        .as_u64()
                        .map(diagnostic_severity_name)
                        .map(|value| Value::String(value.to_owned()))
                        .unwrap_or_else(|| value.clone()),
                    "range" | "targetRange" | "targetSelectionRange" | "selectionRange" => {
                        normalize_range(Some(value))
                    }
                    "position" => {
                        let range = json!({ "start": value, "end": value });
                        normalize_range(Some(&range))
                            .get("start")
                            .cloned()
                            .unwrap_or(Value::Null)
                    }
                    _ => normalize_lsp_value(root, value),
                };
                normalized.insert(mapped_key.to_owned(), mapped_value);
                if value
                    .as_array()
                    .is_some_and(|items| items.len() > MAX_RESULTS)
                {
                    normalized.insert(format!("{mapped_key}_truncated"), Value::Bool(true));
                }
            }
            Value::Object(normalized)
        }
        _ => value.clone(),
    }
}

pub(super) fn symbol_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "file",
        2 | 3 => "module",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        15 => "string",
        16 => "number",
        17 => "boolean",
        18 => "array",
        19 => "object",
        20 => "key",
        21 => "null",
        22 => "enum_member",
        23 => "struct",
        24 => "event",
        25 => "operator",
        26 => "type_parameter",
        _ => "other",
    }
}

#[cfg(test)]
pub(super) fn flycheck_result_json(mut diagnostics: Value) -> Value {
    let items = diagnostics
        .get("diagnostics")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let error_count = items
        .iter()
        .filter(|item| item.get("severity").and_then(Value::as_str) == Some("error"))
        .count();
    let warning_count = items
        .iter()
        .filter(|item| item.get("severity").and_then(Value::as_str) == Some("warning"))
        .count();
    let Some(object) = diagnostics.as_object_mut() else {
        return json!({
            "operation": "cargo_clippy",
            "command": "clippy",
            "warnings_denied": true,
            "status": "completed",
            "cargo_exit_status": "not_reported_by_rust_analyzer",
            "diagnostic_summary": { "error_count": error_count, "warning_count": warning_count },
            "diagnostics": diagnostics,
            "truncated": false
        });
    };
    object.insert("operation".to_owned(), json!("cargo_clippy"));
    object.insert("command".to_owned(), json!("clippy"));
    object.insert("warnings_denied".to_owned(), json!(true));
    object.insert(
        "status".to_owned(),
        json!(if error_count > 0 || warning_count > 0 {
            "completed_with_errors"
        } else {
            "completed"
        }),
    );
    object.insert(
        "cargo_exit_status".to_owned(),
        json!("not_reported_by_rust_analyzer"),
    );
    object.insert(
        "diagnostic_summary".to_owned(),
        json!({ "error_count": error_count, "warning_count": warning_count }),
    );
    diagnostics
}
