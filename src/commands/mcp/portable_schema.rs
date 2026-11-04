//! Rewrite generated tool schemas into a portable JSON Schema subset.
//!
//! schemars renders `Option<T>` as `"type": ["T", "null"]`. That is valid
//! JSON Schema, but clients that map tool schemas onto a single-`type`
//! dialect (for example Gemini function declarations) may reject the tool or
//! drop its constraints, and MCP Inspector reports every such field.
//!
//! An optional property (one not listed in `required`) collapses to its
//! single non-null type, because omitting it already means "no value"; serde
//! still accepts an explicit `null` for those fields. Any other nullable type
//! becomes `anyOf` branches that each carry a single `type`.

use std::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::model::JsonObject;
use serde_json::{Map, Value, json};

/// Normalize the input and output schemas of every route in `router`.
pub(super) fn portable_router<S>(mut router: ToolRouter<S>) -> ToolRouter<S> {
    for route in router.map.values_mut() {
        route.attr.input_schema = portable(&route.attr.input_schema);
        if let Some(output) = &route.attr.output_schema {
            route.attr.output_schema = Some(portable(output));
        }
    }
    router
}

fn portable(schema: &Arc<JsonObject>) -> Arc<JsonObject> {
    let mut schema = JsonObject::clone(schema);
    normalize(&mut schema, false);
    Arc::new(schema)
}

/// Normalize one schema object. `optional` is true for a property that the
/// parent object does not list in `required`.
fn normalize(schema: &mut Map<String, Value>, optional: bool) {
    rewrite_nullable_type(schema, optional);

    let required = required_names(schema);
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        for (name, property) in properties.iter_mut() {
            if let Value::Object(property) = property {
                normalize(property, !required.contains(name));
            }
        }
    }
    for key in ["$defs", "definitions", "patternProperties"] {
        if let Some(Value::Object(children)) = schema.get_mut(key) {
            children.values_mut().for_each(normalize_value);
        }
    }
    for key in ["items", "additionalProperties", "not", "if", "then", "else"] {
        if let Some(child) = schema.get_mut(key) {
            normalize_value(child);
        }
    }
    for key in ["anyOf", "oneOf", "allOf", "prefixItems"] {
        if let Some(Value::Array(children)) = schema.get_mut(key) {
            children.iter_mut().for_each(normalize_value);
        }
    }
}

fn normalize_value(value: &mut Value) {
    if let Value::Object(schema) = value {
        normalize(schema, false);
    }
}

fn required_names(schema: &Map<String, Value>) -> Vec<String> {
    match schema.get("required") {
        Some(Value::Array(names)) => names
            .iter()
            .filter_map(|name| name.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

fn rewrite_nullable_type(schema: &mut Map<String, Value>, optional: bool) {
    let Some(Value::Array(types)) = schema.get("type") else {
        return;
    };
    let non_null = types
        .iter()
        .filter(|kind| kind.as_str() != Some("null"))
        .cloned()
        .collect::<Vec<_>>();
    let nullable = non_null.len() < types.len();

    match (optional && nullable, non_null.as_slice()) {
        (true, [single]) => {
            schema.insert("type".to_owned(), single.clone());
            if schema.get("default") == Some(&Value::Null) {
                schema.remove("default");
            }
        }
        _ if !schema.contains_key("anyOf") => {
            let types = if optional && nullable {
                non_null
            } else {
                types.clone()
            };
            schema.remove("type");
            if optional && schema.get("default") == Some(&Value::Null) {
                schema.remove("default");
            }
            let branches = types
                .into_iter()
                .map(|kind| json!({ "type": kind }))
                .collect();
            schema.insert("anyOf".to_owned(), Value::Array(branches));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalized(schema: Value) -> Value {
        let Value::Object(mut schema) = schema else {
            panic!("schema must be an object");
        };
        normalize(&mut schema, false);
        Value::Object(schema)
    }

    #[test]
    fn optional_nullable_property_collapses_to_its_single_type() {
        let schema = normalized(json!({
            "type": "object",
            "properties": {
                "limit": { "type": ["integer", "null"], "default": null, "minimum": 1 }
            }
        }));
        assert_eq!(
            schema["properties"]["limit"],
            json!({ "type": "integer", "minimum": 1 })
        );
    }

    #[test]
    fn required_nullable_property_becomes_single_type_branches() {
        let schema = normalized(json!({
            "type": "object",
            "required": ["file"],
            "properties": { "file": { "type": ["string", "null"] } }
        }));
        assert_eq!(
            schema["properties"]["file"],
            json!({ "anyOf": [{ "type": "string" }, { "type": "null" }] })
        );
    }

    #[test]
    fn nested_definitions_and_items_are_normalized() {
        let schema = normalized(json!({
            "type": "object",
            "properties": {
                "edits": { "type": "array", "items": { "$ref": "#/$defs/Edit" } }
            },
            "$defs": {
                "Edit": {
                    "type": "object",
                    "required": ["path"],
                    "properties": {
                        "path": { "type": "string" },
                        "new_text": { "type": ["string", "null"], "default": null }
                    }
                }
            }
        }));
        assert_eq!(
            schema["$defs"]["Edit"]["properties"]["new_text"],
            json!({ "type": "string" })
        );
    }

    #[test]
    fn single_types_are_left_untouched() {
        let original = json!({
            "type": "object",
            "properties": { "include_tests": { "type": "boolean", "default": false } }
        });
        assert_eq!(normalized(original.clone()), original);
    }

    #[test]
    fn every_registered_tool_schema_uses_single_types() {
        let router = super::super::AimerMcpServer::tool_router();
        for tool in router.list_all() {
            let schema = Value::Object(JsonObject::clone(&tool.input_schema));
            assert_no_type_arrays(&tool.name, &schema);
        }
    }

    fn assert_no_type_arrays(tool: &str, value: &Value) {
        match value {
            Value::Object(map) => {
                assert!(
                    !matches!(map.get("type"), Some(Value::Array(_))),
                    "tool `{tool}` still has a `type` array: {value}"
                );
                map.values()
                    .for_each(|child| assert_no_type_arrays(tool, child));
            }
            Value::Array(items) => items
                .iter()
                .for_each(|child| assert_no_type_arrays(tool, child)),
            _ => {}
        }
    }
}
