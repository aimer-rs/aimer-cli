use serde_json::{Map, Value};

const LINK_KEYS: [&str; 4] = [
    "target_file",
    "target_range",
    "target_selection_range",
    "origin_selection_range",
];

/// Format a search or navigation result item as `file:line[:column] label`.
///
/// Recognised shapes: text hits (`file`, `line`, `column`, `text`), workspace
/// symbols (`name`, `kind`, `location`), implementations (`file`, `line`,
/// `header`), and LSP locations or location links (`file`/`target_file` plus a
/// range). Returns `None` for any other shape.
pub(super) fn location_line(item: &Value) -> Option<String> {
    let map = item.as_object()?;
    text_hit(map)
        .or_else(|| symbol(map))
        .or_else(|| implementation(map))
        .or_else(|| lsp_location(map))
}

fn text_hit(map: &Map<String, Value>) -> Option<String> {
    if map.len() != 4 {
        return None;
    }
    let file = map.get("file")?.as_str()?;
    let line = map.get("line")?.as_u64()?;
    let column = map.get("column")?.as_u64()?;
    let text = map.get("text")?.as_str()?;
    Some(format!("{file}:{line}:{column} {text}"))
}

fn symbol(map: &Map<String, Value>) -> Option<String> {
    let name = map.get("name")?.as_str()?;
    let kind = map.get("kind")?.as_str()?;
    let location = map.get("location")?.as_object()?;
    let place = place(location.get("file")?.as_str()?, location.get("range")?)?;
    let mut line = format!("{place} {kind} {name}");
    if let Some(path) = map
        .get("module_path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
    {
        line.push_str(&format!(" ({path})"));
    }
    if map.get("is_reexport").and_then(Value::as_bool) == Some(true) {
        line.push_str(" [reexport]");
    }
    Some(line)
}

fn implementation(map: &Map<String, Value>) -> Option<String> {
    let file = map.get("file")?.as_str()?;
    let line = map.get("line")?.as_u64()?;
    let header = map.get("header")?.as_str()?;
    let mut text = format!("{file}:{line} {header}");
    if map.get("in_test_cfg").and_then(Value::as_bool) == Some(true) {
        text.push_str(" [test]");
    }
    Some(text)
}

fn lsp_location(map: &Map<String, Value>) -> Option<String> {
    if let Some(file) = map.get("target_file").and_then(Value::as_str) {
        if !map.keys().all(|key| LINK_KEYS.contains(&key.as_str())) {
            return None;
        }
        let range = map
            .get("target_selection_range")
            .or_else(|| map.get("target_range"))?;
        return place(file, range);
    }
    if !map.keys().all(|key| key == "file" || key == "range") {
        return None;
    }
    place(map.get("file")?.as_str()?, map.get("range")?)
}

fn place(file: &str, range: &Value) -> Option<String> {
    let start = range.get("start")?;
    let line = start.get("line")?.as_u64()?;
    let column = start.get("column")?.as_u64()?;
    Some(format!("{file}:{line}:{column}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn range(line: u64, column: u64) -> Value {
        json!({
            "start": { "line": line, "column": column },
            "end": { "line": line, "column": column + 1 }
        })
    }

    #[test]
    fn text_hit_keeps_its_text() {
        let item = json!({ "file": "a.rs", "line": 3, "column": 4, "text": "x | y" });
        assert_eq!(location_line(&item).unwrap(), "a.rs:3:4 x | y");
    }

    #[test]
    fn lsp_location_and_link_use_start_position() {
        let location = json!({ "file": "src/lib.rs", "range": range(10, 5) });
        assert_eq!(location_line(&location).unwrap(), "src/lib.rs:10:5");

        let link = json!({
            "target_file": "src/b.rs",
            "target_range": range(1, 1),
            "target_selection_range": range(7, 12),
            "origin_selection_range": range(2, 2)
        });
        assert_eq!(location_line(&link).unwrap(), "src/b.rs:7:12");
    }

    #[test]
    fn symbol_shows_kind_name_and_module_path() {
        let symbol = json!({
            "name": "Widget",
            "kind": "struct",
            "location": { "file": "src/w.rs", "range": range(4, 12) },
            "match_quality": "exact",
            "crate": "demo",
            "module_path": "demo::w",
            "is_reexport": true
        });
        assert_eq!(
            location_line(&symbol).unwrap(),
            "src/w.rs:4:12 struct Widget (demo::w) [reexport]"
        );
    }

    #[test]
    fn implementation_has_line_but_no_column() {
        let item = json!({
            "name": "Zed",
            "header": "impl Drawable for Zed",
            "crate": "demo",
            "file": "src/lib.rs",
            "line": 2,
            "in_test_cfg": true
        });
        assert_eq!(
            location_line(&item).unwrap(),
            "src/lib.rs:2 impl Drawable for Zed [test]"
        );
    }

    #[test]
    fn unknown_shapes_are_not_locations() {
        assert!(
            location_line(&json!({ "file": "a.rs", "range": range(1, 1), "extra": 1 })).is_none()
        );
        assert!(location_line(&json!({ "name": "x", "kind": "function" })).is_none());
        assert!(location_line(&json!("text")).is_none());
    }
}
