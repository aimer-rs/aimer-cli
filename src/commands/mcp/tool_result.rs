use super::*;

pub(super) fn encode(operation: &str, result: anyhow::Result<Value>) -> CallToolResult {
    match result {
        Ok(value @ Value::Object(_)) => CallToolResult::structured(value),
        Ok(value) => CallToolResult::structured(json!({ "result": value })),
        Err(error) => {
            let message = format!("{error:#}");
            let lower_message = message.to_ascii_lowercase();
            let (code, phase) = if operation == "check"
                && (lower_message.contains("check_start_timeout")
                    || lower_message.contains("did not start flycheck"))
            {
                ("check_start_timeout", "start")
            } else if operation == "check" && lower_message.contains("check_runner_unavailable") {
                ("check_runner_unavailable", "runner")
            } else if operation == "check"
                && (lower_message.contains("check_timeout")
                    || lower_message.contains("flycheck timed out")
                    || lower_message.contains("timed out waiting for rust-analyzer"))
            {
                ("check_timeout", "execution")
            } else if lower_message.contains("no live aimer session is attached") {
                ("session_unavailable", "attachment")
            } else if lower_message.contains("did not report a ready workspace")
                || (lower_message.contains("workspace") && lower_message.contains("index"))
            {
                ("workspace_not_ready", "indexing")
            } else if lower_message.contains("starting rust-analyzer")
                || lower_message.contains("could not start rust-analyzer")
            {
                ("lsp_start_failed", "startup")
            } else if lower_message.contains("timed out") || lower_message.contains("timeout") {
                ("timeout", "execution")
            } else if lower_message.contains("must not be empty")
                || lower_message.contains("must be")
                || lower_message.contains("invalid")
                || lower_message.contains("unknown rust symbol kind")
            {
                ("invalid_argument", "validation")
            } else if lower_message.contains("rust-analyzer") {
                ("lsp_error", "lsp_request")
            } else {
                ("operation_failed", "execution")
            };
            CallToolResult::structured_error(json!({
                "error": {
                    "code": code,
                    "operation": operation,
                    "phase": phase,
                    "message": message
                }
            }))
        }
    }
}

pub(super) fn encode_inspect_file(
    result: anyhow::Result<Value>,
    format: Option<&str>,
) -> CallToolResult {
    match (
        format.unwrap_or("json").to_ascii_lowercase().as_str(),
        result,
    ) {
        ("json", Ok(value)) => {
            let mut result = CallToolResult::structured(value);
            result.content.clear();
            result
        }
        ("text", Ok(value)) => {
            CallToolResult::success(vec![ContentBlock::text(inspect_file_text(&value))])
        }
        ("json" | "text", Err(error)) => encode("inspect_file", Err(error)),
        (other, _) => encode(
            "inspect_file",
            Err(anyhow!("format must be 'json' or 'text', got '{other}'")),
        ),
    }
}

fn inspect_file_text(value: &Value) -> String {
    let mut lines = Vec::new();
    if let Some(file) = value.get("file").and_then(Value::as_str) {
        lines.push(format!("file: {file}"));
    }
    if let Some(ranges) = value.get("ranges").and_then(Value::as_array) {
        for range in ranges {
            let start = range
                .get("start_line")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let text = range
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            lines.push(format!("\nsource lines starting at {start}:"));
            for (offset, line) in text.lines().enumerate() {
                lines.push(format!("{:>5} | {line}", start + offset as u64));
            }
        }
    }
    if let Some(searches) = value.get("searches").and_then(Value::as_array) {
        for search in searches {
            let pattern = search
                .get("pattern")
                .and_then(Value::as_str)
                .unwrap_or_default();
            lines.push(format!("\nsearch: {pattern}"));
            if let Some(matches) = search.get("matches").and_then(Value::as_array) {
                for matched in matches {
                    let line_number = matched
                        .get("line")
                        .and_then(Value::as_u64)
                        .unwrap_or_default();
                    let before = matched
                        .get("context_before")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .lines()
                        .collect::<Vec<_>>();
                    let after = matched
                        .get("context_after")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .lines()
                        .collect::<Vec<_>>();
                    for (offset, text) in before.iter().enumerate() {
                        lines.push(format!(
                            "{:>5} | {text}",
                            line_number.saturating_sub(before.len() as u64) + offset as u64
                        ));
                    }
                    lines.push(format!(
                        "{:>5} | {}",
                        line_number,
                        matched
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                    ));
                    for (offset, text) in after.iter().enumerate() {
                        lines.push(format!("{:>5} | {text}", line_number + offset as u64 + 1));
                    }
                }
            }
        }
    }
    if value.get("truncated").and_then(Value::as_bool) == Some(true) {
        lines.push("\n[output truncated]".to_owned());
    }
    if let Some(truncated_lines) = value.get("truncated_lines").and_then(Value::as_array)
        && !truncated_lines.is_empty()
    {
        let numbers = truncated_lines
            .iter()
            .filter_map(Value::as_u64)
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("[truncated source lines: {numbers}]"));
    }
    lines.join("\n")
}

pub(super) fn mcp_result_schema() -> Arc<Map<String, Value>> {
    let mut schema = Map::new();
    schema.insert("type".to_owned(), json!("object"));
    schema.insert("additionalProperties".to_owned(), json!(true));
    Arc::new(schema)
}
