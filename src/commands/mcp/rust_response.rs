use super::*;

pub(super) fn validate_rust_limit(limit: Option<usize>) -> anyhow::Result<()> {
    if limit.is_some_and(|limit| !(1..=200).contains(&limit)) {
        anyhow::bail!("limit must be between 1 and 200");
    }
    Ok(())
}

pub(super) fn normalize_completion(
    root: &Path,
    value: &Value,
    limit: usize,
    include_documentation: bool,
) -> Value {
    const MAX_COMPLETION_DOCUMENTATION_CHARS: usize = 1_000;
    const MAX_COMPLETION_DOCUMENTATION_TOTAL_CHARS: usize = 8_000;

    fn bound_documentation(item: &mut Value, remaining: &mut usize) -> bool {
        let Some(text) = item
            .get("documentation")
            .and_then(|documentation| match documentation {
                Value::String(text) => Some(text.as_str()),
                Value::Object(object) => object.get("value").and_then(Value::as_str),
                _ => None,
            })
            .map(str::to_owned)
        else {
            return false;
        };
        let original_chars = text.chars().count();
        if original_chars == 0 {
            if let Some(object) = item.as_object_mut() {
                object.remove("documentation");
            }
            return false;
        }
        let allowed = MAX_COMPLETION_DOCUMENTATION_CHARS.min(*remaining);
        if allowed == 0 {
            if let Some(object) = item.as_object_mut() {
                object.remove("documentation");
            }
            return true;
        }
        let truncated = original_chars > allowed;
        if truncated {
            let mut shortened = text.chars().take(allowed).collect::<String>();
            shortened.push('…');
            if let Some(documentation) = item
                .as_object_mut()
                .and_then(|object| object.get_mut("documentation"))
            {
                match documentation {
                    Value::String(value) => *value = shortened,
                    Value::Object(object) => {
                        object.insert("value".to_owned(), Value::String(shortened));
                    }
                    _ => {}
                }
            }
        }
        *remaining = remaining.saturating_sub(original_chars.min(allowed));
        truncated
    }

    let items = value
        .as_array()
        .or_else(|| value.get("items").and_then(Value::as_array))
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut documentation_remaining = MAX_COMPLETION_DOCUMENTATION_TOTAL_CHARS;
    let mut documentation_truncated = false;
    let results = items
        .iter()
        .take(limit)
        .map(|item| {
            let kind = item.get("kind").and_then(Value::as_u64);
            let mut normalized = normalize_result(root, item);
            if let (Some(kind), Some(object)) = (kind, normalized.as_object_mut()) {
                object.insert(
                    "kind".to_owned(),
                    Value::String(completion_kind_name(kind).to_owned()),
                );
            }
            if include_documentation {
                documentation_truncated |=
                    bound_documentation(&mut normalized, &mut documentation_remaining);
            } else if let Some(object) = normalized.as_object_mut() {
                object.remove("documentation");
            }
            normalized
        })
        .collect::<Vec<_>>();
    let mut result = json!({ "results": results });
    if value
        .get("isIncomplete")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        result["is_incomplete"] = Value::Bool(true);
    }
    if items.len() > limit {
        result["truncated"] = Value::Bool(true);
    }
    if documentation_truncated {
        result["documentation_truncated"] = Value::Bool(true);
    }
    result
}

pub(super) fn normalize_document_highlights(root: &Path, value: &Value) -> Value {
    let items = value.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let (results, truncated) =
        normalize_lsp_items(root, items, document_highlight_kind_name, false);
    let mut result = json!({ "results": results });
    if truncated {
        result["truncated"] = Value::Bool(true);
    }
    result
}

pub(super) fn normalize_prepare_rename(root: &Path, value: &Value) -> Value {
    if value.get("range").is_some() || value.get("defaultBehavior").is_some() {
        return normalize_result(root, value);
    }
    if value.get("start").is_some() && value.get("end").is_some() {
        return normalize_result(root, &json!({ "range": value }))
            .get("range")
            .cloned()
            .unwrap_or(Value::Null);
    }
    normalize_result(root, value)
}

pub(super) fn normalize_inlay_hints(root: &Path, value: &Value) -> Value {
    let items = value.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let (results, truncated) = normalize_lsp_items(root, items, inlay_hint_kind_name, false);
    let mut result = json!({ "results": results });
    if truncated {
        result["truncated"] = Value::Bool(true);
    }
    result
}

pub(super) fn normalize_lsp_items(
    root: &Path,
    items: &[Value],
    kind_name: fn(u64) -> &'static str,
    truncate_documentation: bool,
) -> (Vec<Value>, bool) {
    let truncated = items.len() > 200;
    let results = items
        .iter()
        .take(200)
        .map(|item| {
            let kind = item.get("kind").and_then(Value::as_u64);
            let mut normalized = normalize_result(root, item);
            if let (Some(kind), Some(object)) = (kind, normalized.as_object_mut()) {
                object.insert("kind".to_owned(), Value::String(kind_name(kind).to_owned()));
            }
            if truncate_documentation {
                truncate_completion_documentation(&mut normalized);
            }
            normalized
        })
        .collect();
    (results, truncated)
}

pub(super) fn truncate_completion_documentation(item: &mut Value) {
    const MAX_DOCUMENTATION_CHARS: usize = 8_000;
    let truncated = if let Some(documentation) = item.get_mut("documentation") {
        match documentation {
            Value::String(text) => truncate_text(text, MAX_DOCUMENTATION_CHARS),
            Value::Object(object) => match object.get_mut("value") {
                Some(Value::String(text)) => truncate_text(text, MAX_DOCUMENTATION_CHARS),
                _ => false,
            },
            _ => false,
        }
    } else {
        false
    };
    if truncated {
        if let Some(object) = item.as_object_mut() {
            object.insert("documentation_truncated".to_owned(), Value::Bool(true));
        }
    }
}

pub(super) fn truncate_text(text: &mut String, max_chars: usize) -> bool {
    if text.chars().count() <= max_chars {
        return false;
    }
    *text = text.chars().take(max_chars).collect();
    text.push('…');
    true
}

pub(super) fn completion_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "text",
        2 => "method",
        3 => "function",
        4 => "constructor",
        5 => "field",
        6 => "variable",
        7 => "class",
        8 => "interface",
        9 => "module",
        10 => "property",
        11 => "unit",
        12 => "value",
        13 => "enum",
        14 => "keyword",
        15 => "snippet",
        16 => "color",
        17 => "file",
        18 => "reference",
        19 => "folder",
        20 => "enum_member",
        21 => "constant",
        22 => "struct",
        23 => "event",
        24 => "operator",
        25 => "type_parameter",
        _ => "other",
    }
}

pub(super) fn document_highlight_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "text",
        2 => "read",
        3 => "write",
        _ => "other",
    }
}

pub(super) fn inlay_hint_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "type",
        2 => "parameter",
        _ => "other",
    }
}

pub(super) fn rust_symbol_is_exact_match(symbol: &Value, query: &str) -> bool {
    let query = query.trim().trim_start_matches("::");
    let name = symbol
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if name.eq_ignore_ascii_case(query) {
        return true;
    }

    let container = symbol
        .get("containerName")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if container.is_empty() || name.is_empty() {
        return false;
    }

    let full_path = format!("{container}::{name}");
    let query = query.to_ascii_lowercase();
    let full_path = full_path.to_ascii_lowercase();
    query == full_path || query.ends_with(&format!("::{full_path}"))
}

pub(super) fn rust_symbol_sort_key(
    symbol: &Value,
    query: &str,
) -> (bool, String, String, String, u64, u64, u64, String) {
    let name = symbol
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let container = symbol
        .get("containerName")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let location = symbol.get("location");
    let uri = location
        .and_then(|location| location.get("uri"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let start = location
        .and_then(|location| location.get("range"))
        .and_then(|range| range.get("start"));

    (
        !rust_symbol_is_exact_match(symbol, query),
        name.to_ascii_lowercase(),
        container.to_ascii_lowercase(),
        uri.to_owned(),
        start
            .and_then(|position| position.get("line"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        start
            .and_then(|position| position.get("character"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        symbol
            .get("kind")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        serde_json::to_string(symbol).unwrap_or_default(),
    )
}

#[cfg(test)]
pub(super) fn sort_rust_symbols(symbols: &mut [Value], query: &str) {
    symbols.sort_by_cached_key(|symbol| rust_symbol_sort_key(symbol, query));
}

pub(super) fn rust_symbol_kind_name(kind: u64) -> &'static str {
    match kind {
        2 | 3 => "module",
        5 => "class",
        6 => "method",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        22 => "enum_member",
        23 => "struct",
        25 => "operator",
        26 => "type_parameter",
        _ => "other",
    }
}

pub(super) fn normalize_rust_symbol_kind(kind: &str) -> anyhow::Result<String> {
    let kind = kind.trim().to_ascii_lowercase().replace('-', "_");
    let normalized = match kind.as_str() {
        "interface" => "trait",
        "class" => "struct",
        "enum_member" => "variant",
        "value" => "variable",
        "function" | "struct" | "enum" | "trait" | "module" | "method" | "field" | "variable"
        | "constant" | "type_parameter" | "variant" | "macro" | "impl" => &kind,
        _ => anyhow::bail!(
            "unknown Rust symbol kind '{kind}'; valid kinds: function, struct, enum, trait, impl, macro, module, method, field, variable, constant, type_parameter, variant"
        ),
    };
    Ok(normalized.to_owned())
}
