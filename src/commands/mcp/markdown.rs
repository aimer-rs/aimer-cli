use serde_json::{Map, Value};

use super::locations::location_line;

const MAX_TABLE_COLUMNS: usize = 12;
const MAX_INLINE_LIST_CHARS: usize = 300;
const MAX_INLINE_OBJECT_CHARS: usize = 120;

pub(super) fn to_markdown(value: &Value) -> String {
    let mut lines = Vec::new();
    match value {
        Value::Null => lines.push("ok".to_owned()),
        Value::Object(map) if map.is_empty() => lines.push("ok".to_owned()),
        Value::Object(map) => match location_line(value) {
            Some(line) => lines.push(format!("- {line}")),
            None => render_object(map, 0, &mut lines),
        },
        Value::Array(items) if items.is_empty() => lines.push("(empty)".to_owned()),
        Value::Array(items) => render_array(items, 0, &mut lines),
        Value::String(text) => lines.push(text.clone()),
        other => lines.push(other.to_string()),
    }
    lines.join("\n")
}

pub(super) fn error_markdown(code: &str, operation: &str, phase: &str, message: &str) -> String {
    format!("**Error `{code}`** ({operation}, phase: {phase}): {message}")
}

fn pad(depth: usize) -> String {
    "  ".repeat(depth)
}

fn is_scalar(value: &Value) -> bool {
    !matches!(value, Value::Array(_) | Value::Object(_))
}

fn is_inline_scalar(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.contains('\n'),
        other => is_scalar(other),
    }
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::String(text) if text.is_empty() => "\"\"".to_owned(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn render_object(map: &Map<String, Value>, depth: usize, lines: &mut Vec<String>) {
    for (key, value) in map {
        if value.is_null() {
            continue;
        }
        if let Some(hits) = value
            .as_array()
            .and_then(|items| location_lines(items, depth))
        {
            lines.extend(hits);
            continue;
        }
        render_field(key, value, depth, lines);
    }
}

fn render_field(key: &str, value: &Value, depth: usize, lines: &mut Vec<String>) {
    let indent = pad(depth);
    match value {
        Value::String(text) if text.contains('\n') => {
            lines.push(format!("{indent}- {key}:"));
            push_fenced(text, depth + 1, lines);
        }
        scalar if is_scalar(scalar) => {
            lines.push(format!("{indent}- {key}: {}", scalar_text(scalar)));
        }
        Value::Array(items) if items.is_empty() => lines.push(format!("{indent}- {key}: []")),
        Value::Object(map) if map.is_empty() => lines.push(format!("{indent}- {key}: {{}}")),
        Value::Array(items) => {
            if let Some(inline) = inline_list(items) {
                lines.push(format!("{indent}- {key}: {inline}"));
            } else {
                lines.push(format!("{indent}- {key}:"));
                render_array(items, depth + 1, lines);
            }
        }
        Value::Object(map) => {
            if let Some(inline) = inline_object(map) {
                lines.push(format!("{indent}- {key}: {inline}"));
            } else {
                lines.push(format!("{indent}- {key}:"));
                render_object(map, depth + 1, lines);
            }
        }
        _ => unreachable!("scalars are handled above"),
    }
}

/// `key=value, key=value` for a small object of single-line scalars.
fn inline_object(map: &Map<String, Value>) -> Option<String> {
    let mut parts = Vec::new();
    for (key, value) in map {
        if value.is_null() {
            continue;
        }
        if !is_inline_scalar(value) {
            return None;
        }
        parts.push(format!("{key}={}", scalar_text(value)));
    }
    if parts.is_empty() {
        return None;
    }
    let joined = parts.join(", ");
    (joined.chars().count() <= MAX_INLINE_OBJECT_CHARS).then_some(joined)
}

fn inline_list(items: &[Value]) -> Option<String> {
    if !items
        .iter()
        .all(|item| !item.is_null() && is_inline_scalar(item))
    {
        return None;
    }
    let joined = items.iter().map(scalar_text).collect::<Vec<_>>().join(", ");
    (joined.chars().count() <= MAX_INLINE_LIST_CHARS).then_some(joined)
}

fn render_array(items: &[Value], depth: usize, lines: &mut Vec<String>) {
    if let Some(table) = table_lines(items, depth) {
        lines.extend(table);
        return;
    }
    let indent = pad(depth);
    for (index, item) in items.iter().enumerate() {
        match item {
            Value::String(text) if text.contains('\n') => {
                lines.push(format!("{indent}-"));
                push_fenced(text, depth + 1, lines);
            }
            scalar if is_scalar(scalar) => {
                lines.push(format!("{indent}- {}", scalar_text(scalar)));
            }
            Value::Array(inner) if inner.is_empty() => lines.push(format!("{indent}- []")),
            Value::Object(map) if map.is_empty() => lines.push(format!("{indent}- {{}}")),
            Value::Array(inner) => {
                if let Some(inline) = inline_list(inner) {
                    lines.push(format!("{indent}- {inline}"));
                } else {
                    lines.push(format!("{indent}- [{index}]"));
                    render_array(inner, depth + 1, lines);
                }
            }
            Value::Object(map) => {
                if let Some(inline) = inline_object(map) {
                    lines.push(format!("{indent}- {inline}"));
                } else {
                    lines.push(format!("{indent}- [{index}]"));
                    render_object(map, depth + 1, lines);
                }
            }
            _ => unreachable!("scalars are handled above"),
        }
    }
}

/// Render search and navigation results (text hits, symbols, implementations,
/// locations) as `- file:line:column ...` bullets instead of a table.
fn location_lines(items: &[Value], depth: usize) -> Option<Vec<String>> {
    let indent = pad(depth);
    items
        .iter()
        .map(|item| location_line(item).map(|line| format!("{indent}- {line}")))
        .collect::<Option<Vec<_>>>()
        .filter(|hits| !hits.is_empty())
}

fn table_lines(items: &[Value], depth: usize) -> Option<Vec<String>> {
    if items.len() < 2 {
        return None;
    }
    let mut columns: Vec<&str> = Vec::new();
    for item in items {
        let map = item.as_object()?;
        if map.is_empty() || !map.values().all(is_inline_scalar) {
            return None;
        }
        for (key, value) in map {
            if !value.is_null() && !columns.contains(&key.as_str()) {
                columns.push(key);
            }
        }
    }
    if columns.is_empty() || columns.len() > MAX_TABLE_COLUMNS {
        return None;
    }
    let indent = pad(depth);
    let mut lines = vec![
        format!("{indent}| {} |", columns.join(" | ")),
        format!("{indent}|{}", "---|".repeat(columns.len())),
    ];
    for item in items {
        let map = item.as_object()?;
        let cells = columns
            .iter()
            .map(|column| match map.get(*column) {
                None | Some(Value::Null) => String::new(),
                Some(value) => scalar_text(value).replace('|', "\\|"),
            })
            .collect::<Vec<_>>();
        lines.push(format!("{indent}| {} |", cells.join(" | ")));
    }
    Some(lines)
}

fn push_fenced(text: &str, depth: usize, lines: &mut Vec<String>) {
    let indent = pad(depth);
    let longest_run = text
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest_run.max(2) + 1);
    lines.push(format!("{indent}{fence}"));
    lines.extend(text.lines().map(|line| format!("{indent}{line}")));
    lines.push(format!("{indent}{fence}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn objects_render_as_bullets_and_skip_nulls() {
        let markdown = to_markdown(&json!({
            "a_name": "demo",
            "b_count": 2,
            "c_missing": null,
            "d_tags": ["a", "b"],
            "e_nested": { "ready": true }
        }));
        assert_eq!(
            markdown,
            "- a_name: demo\n- b_count: 2\n- d_tags: a, b\n- e_nested: ready=true"
        );
    }

    #[test]
    fn uniform_object_arrays_render_as_tables() {
        let markdown = to_markdown(&json!({
            "results": [
                { "a_name": "A", "b_line": 1 },
                { "a_name": "B|C", "b_line": 2 }
            ]
        }));
        assert_eq!(
            markdown,
            "- results:\n  | a_name | b_line |\n  |---|---|\n  | A | 1 |\n  | B\\|C | 2 |"
        );
    }

    #[test]
    fn search_hits_render_as_location_bullets_not_tables() {
        let markdown = to_markdown(&json!({
            "query": "join",
            "results": [
                { "file": "a/src/tests.rs", "line": 31, "column": 27, "text": "project.join(\"src/main.rs\")" },
                { "file": "b.rs", "line": 2, "column": 1, "text": "a | b" }
            ],
            "truncated": true
        }));
        assert_eq!(
            markdown,
            "- query: join\n- a/src/tests.rs:31:27 project.join(\"src/main.rs\")\n- b.rs:2:1 a | b\n- truncated: true"
        );
    }

    #[test]
    fn symbol_and_location_results_render_as_location_bullets() {
        let range = |line: u64| json!({ "start": { "line": line, "column": 3 } });
        let symbols = to_markdown(&json!({
            "results": [
                { "name": "A", "kind": "struct", "location": { "file": "a.rs", "range": range(1) } },
                { "name": "b", "kind": "function", "location": { "file": "b.rs", "range": range(2) } }
            ],
            "complete": true
        }));
        assert_eq!(
            symbols,
            "- complete: true\n- a.rs:1:3 struct A\n- b.rs:2:3 function b"
        );

        let references = to_markdown(&json!({
            "results": [
                { "file": "a.rs", "range": range(4) },
                { "file": "b.rs", "range": range(5) }
            ]
        }));
        assert_eq!(references, "- a.rs:4:3\n- b.rs:5:3");

        let definition = to_markdown(&json!({ "file": "a.rs", "range": range(9) }));
        assert_eq!(definition, "- a.rs:9:3");
    }

    #[test]
    fn multiline_strings_use_fences_longer_than_embedded_backticks() {
        let markdown = to_markdown(&json!({ "output": "a\n```\nb" }));
        assert_eq!(markdown, "- output:\n  ````\n  a\n  ```\n  b\n  ````");
    }

    #[test]
    fn nested_objects_in_arrays_get_indexed_entries() {
        let markdown = to_markdown(&json!({ "items": [{ "a": { "b": 1 } }, { "a": 2 }] }));
        assert_eq!(markdown, "- items:\n  - [0]\n    - a: b=1\n  - a=2");
    }

    #[test]
    fn empty_values_are_explicit() {
        assert_eq!(to_markdown(&Value::Null), "ok");
        assert_eq!(
            to_markdown(&json!({ "a_results": [], "b_meta": {} })),
            "- a_results: []\n- b_meta: {}"
        );
        assert_eq!(to_markdown(&json!([])), "(empty)");
    }

    #[test]
    fn errors_are_one_compact_line() {
        assert_eq!(
            error_markdown("check_timeout", "check", "execution", "took too long"),
            "**Error `check_timeout`** (check, phase: execution): took too long"
        );
    }
}
