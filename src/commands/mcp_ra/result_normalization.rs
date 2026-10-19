use super::*;

pub fn normalize_result(root: &Path, value: &Value) -> Value {
    normalize_lsp_value(root, value)
}

pub fn normalize_bounded(root: &Path, value: &Value) -> Value {
    if let Some(items) = value.as_array() {
        let mut result = json!({
            "results": items.iter().take(MAX_RESULTS).map(|item| normalize_lsp_value(root, item)).collect::<Vec<_>>()
        });
        if items.len() > MAX_RESULTS {
            result["truncated"] = Value::Bool(true);
        }
        result
    } else {
        normalize_lsp_value(root, value)
    }
}

pub fn compact_document_symbols(
    value: &Value,
    max_depth: usize,
    kinds: &[String],
    name_filter: Option<&str>,
    include_variables: bool,
    include_fields: bool,
) -> anyhow::Result<Value> {
    if max_depth > MAX_DOCUMENT_SYMBOL_DEPTH {
        bail!("max_depth must be between 0 and {MAX_DOCUMENT_SYMBOL_DEPTH}");
    }
    let normalized_kinds = kinds
        .iter()
        .map(|kind| normalize_document_symbol_kind(kind))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let items = value.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let mut context = CompactDocumentSymbols {
        remaining: MAX_DOCUMENT_SYMBOLS,
        truncated: false,
        max_depth,
        kinds: &normalized_kinds,
        name_filter: name_filter.map(str::to_ascii_lowercase),
        include_variables,
        include_fields,
    };
    let results = items
        .iter()
        .filter_map(|item| compact_document_symbol(item, &mut context, 0).map(|item| item.0))
        .collect::<Vec<_>>();
    let mut result = json!({ "results": results });
    if context.truncated {
        result["truncated"] = Value::Bool(true);
    }
    Ok(result)
}

struct CompactDocumentSymbols<'a> {
    remaining: usize,
    truncated: bool,
    max_depth: usize,
    kinds: &'a [String],
    name_filter: Option<String>,
    include_variables: bool,
    include_fields: bool,
}

fn compact_document_symbol(
    symbol: &Value,
    context: &mut CompactDocumentSymbols<'_>,
    depth: usize,
) -> Option<(Value, bool)> {
    let kind_number = symbol.get("kind").and_then(Value::as_u64);
    let kind = document_symbol_kind(symbol, kind_number);
    if (kind == "variable" && !context.include_variables)
        || (kind == "field" && !context.include_fields)
    {
        return None;
    }

    let children = symbol.get("children").and_then(Value::as_array);
    let mut compacted_children = Vec::new();
    let mut children_truncated = false;
    if let Some(children) = children.filter(|children| !children.is_empty()) {
        if depth >= context.max_depth {
            children_truncated = children
                .iter()
                .any(|child| document_symbol_subtree_matches(child, context));
            context.truncated |= children_truncated;
        } else {
            for child in children {
                if let Some((child, _)) = compact_document_symbol(child, context, depth + 1) {
                    compacted_children.push(child);
                }
            }
        }
    }

    let name = symbol
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let detail = symbol
        .get("detail")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let matches_kind =
        context.kinds.is_empty() || context.kinds.iter().any(|filter| filter.as_str() == kind);
    let matches_name = context.name_filter.as_ref().is_none_or(|filter| {
        name.to_ascii_lowercase().contains(filter) || detail.to_ascii_lowercase().contains(filter)
    });
    let self_matches = matches_kind && matches_name;
    if !self_matches && compacted_children.is_empty() {
        return None;
    }
    if context.remaining == 0 {
        context.truncated = true;
        return None;
    }
    context.remaining -= 1;

    let mut result = Map::new();
    if let Some(name) = symbol.get("name") {
        result.insert("name".to_owned(), name.clone());
    }
    result.insert("kind".to_owned(), Value::String(kind.to_owned()));
    if let Some(detail) = symbol.get("detail").and_then(Value::as_str) {
        let mut chars = detail.chars();
        let detail = chars
            .by_ref()
            .take(MAX_DOCUMENT_SYMBOL_DETAIL_CHARS)
            .collect::<String>();
        let detail_truncated = chars.next().is_some();
        result.insert("detail".to_owned(), Value::String(detail));
        if detail_truncated {
            result.insert("detail_truncated".to_owned(), Value::Bool(true));
        }
    }
    let range = normalize_range(symbol.get("range").or_else(|| {
        symbol
            .get("location")
            .and_then(|location| location.get("range"))
    }));
    if let Some(start_line) = range
        .get("start")
        .and_then(|position| position.get("line"))
        .and_then(Value::as_u64)
    {
        result.insert("start_line".to_owned(), json!(start_line));
        if let Some(end_line) = range
            .get("end")
            .and_then(|position| position.get("line"))
            .and_then(Value::as_u64)
            .filter(|end_line| *end_line != start_line)
        {
            result.insert("end_line".to_owned(), json!(end_line));
        }
    }
    if symbol.get("deprecated").and_then(Value::as_bool) == Some(true) {
        result.insert("deprecated".to_owned(), Value::Bool(true));
    }
    if let Some(tags) = symbol
        .get("tags")
        .filter(|tags| tags.as_array().is_some_and(|items| !items.is_empty()))
    {
        result.insert("tags".to_owned(), tags.clone());
    }

    if children_truncated {
        result.insert("children_truncated".to_owned(), Value::Bool(true));
    }
    if !compacted_children.is_empty() {
        result.insert("children".to_owned(), Value::Array(compacted_children));
    }
    Some((Value::Object(result), self_matches))
}

fn document_symbol_subtree_matches(symbol: &Value, context: &CompactDocumentSymbols<'_>) -> bool {
    let kind = document_symbol_kind(symbol, symbol.get("kind").and_then(Value::as_u64));
    if (kind == "variable" && !context.include_variables)
        || (kind == "field" && !context.include_fields)
    {
        return false;
    }
    let name = symbol
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let detail = symbol
        .get("detail")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let matches_kind =
        context.kinds.is_empty() || context.kinds.iter().any(|filter| filter.as_str() == kind);
    let matches_name = context.name_filter.as_ref().is_none_or(|filter| {
        name.to_ascii_lowercase().contains(filter) || detail.to_ascii_lowercase().contains(filter)
    });
    (matches_kind && matches_name)
        || symbol
            .get("children")
            .and_then(Value::as_array)
            .is_some_and(|children| {
                children
                    .iter()
                    .any(|child| document_symbol_subtree_matches(child, context))
            })
}

fn document_symbol_kind(symbol: &Value, kind_number: Option<u64>) -> &'static str {
    let name = symbol
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let detail = symbol
        .get("detail")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if name.trim_start().starts_with("impl ") || detail.trim_start().starts_with("impl ") {
        return "impl";
    }
    match kind_number.map(symbol_kind_name).unwrap_or("other") {
        "interface" => "trait",
        "class" => "struct",
        "enum_member" => "variant",
        "value" => "variable",
        kind => kind,
    }
}

fn normalize_document_symbol_kind(kind: &str) -> anyhow::Result<String> {
    let kind = kind.trim().to_ascii_lowercase().replace('-', "_");
    let normalized = match kind.as_str() {
        "interface" => "trait",
        "class" => "struct",
        "enum_member" => "variant",
        "value" => "variable",
        "impl" | "function" | "struct" | "enum" | "trait" | "macro" | "module" | "method"
        | "field" | "variable" | "constant" | "type_parameter" => &kind,
        _ => bail!(
            "unknown Rust symbol kind '{kind}'; valid kinds: function, struct, enum, trait, impl, module, method, field, variable, constant, type_parameter, variant"
        ),
    };
    Ok(normalized.to_owned())
}
