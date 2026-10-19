use super::*;

pub(super) fn now_unix_ms() -> anyhow::Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_millis() as u64)
}

pub(super) fn logs_payload(arguments: LogsArguments) -> Value {
    let mut payload = Map::new();
    if let Some(stream) = arguments.stream.filter(|stream| stream != "all") {
        payload.insert("stream".to_string(), Value::String(stream));
    }
    if let Some(cursor) = arguments.cursor {
        payload.insert("cursor".to_string(), json!(cursor));
    }
    if let Some(run_id) = arguments.run_id {
        payload.insert("run_id".to_string(), json!(run_id));
    }
    if let Some(limit) = arguments.limit {
        payload.insert("limit".to_string(), json!(limit));
    }
    if let Some(include) = arguments.include_source_locations {
        payload.insert("include_source_locations".to_string(), json!(include));
    }
    Value::Object(payload)
}
