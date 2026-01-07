use rmcp::model::Tool;
use serde_json::Value;
use std::fmt::Write;

pub fn generate_typescript_interface(tools: &[Tool], namespace: &str) -> String {
    let mut output = String::new();

    writeln!(
        output,
        "// Auto-generated TypeScript interface for MCP tools"
    )
    .unwrap();
    writeln!(output, "// Do not edit manually\n").unwrap();
    writeln!(output, "declare namespace {namespace} {{").unwrap();

    for tool in tools {
        let interface_name = to_pascal_case(&tool.name);
        let fn_name = tool.name.replace('-', "_");

        if let Some(desc) = &tool.description {
            writeln!(output, "  /** {desc} */").unwrap();
        }

        let params_type = generate_params_interface(&tool.input_schema, &interface_name, 1);
        let return_type = tool
            .output_schema
            .as_ref()
            .map(|schema| {
                json_schema_to_typescript(&serde_json::Value::Object(schema.as_ref().clone()))
            })
            .unwrap_or_else(|| "unknown".to_string());

        if !params_type.is_empty() {
            output.push_str(&params_type);
            writeln!(
                output,
                "  function {fn_name}(params: {interface_name}Params): {return_type};\n"
            )
            .unwrap();
        } else {
            writeln!(output, "  function {fn_name}(): {return_type};\n").unwrap();
        }
    }

    writeln!(output, "}}").unwrap();
    output
}

fn generate_params_interface(
    schema: &serde_json::Map<String, Value>,
    base_name: &str,
    indent: usize,
) -> String {
    let mut output = String::new();
    let indent_str = "  ".repeat(indent);

    let properties = schema.get("properties").and_then(|p| p.as_object());
    let required = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    // Get $defs for reference resolution
    let defs = schema.get("$defs").or_else(|| schema.get("definitions"));

    if let Some(props) = properties {
        if props.is_empty() {
            return String::new();
        }

        writeln!(output, "{indent_str}interface {base_name}Params {{").unwrap();

        for (name, prop_schema) in props {
            let is_required = required.contains(&name.as_str());
            let ts_type = json_schema_to_typescript_with_defs(prop_schema, defs);
            let optional = if is_required { "" } else { "?" };

            if let Some(desc) = prop_schema.get("description").and_then(|d| d.as_str()) {
                writeln!(output, "{indent_str}  /** {desc} */").unwrap();
            }

            writeln!(output, "{indent_str}  {name}{optional}: {ts_type};").unwrap();
        }

        writeln!(output, "{indent_str}}}\n").unwrap();
    }

    output
}

fn json_schema_to_typescript(schema: &Value) -> String {
    // Extract $defs from root schema for reference resolution
    let defs = schema
        .as_object()
        .and_then(|obj| obj.get("$defs").or_else(|| obj.get("definitions")))
        .cloned();

    json_schema_to_typescript_with_defs(schema, defs.as_ref())
}

fn json_schema_to_typescript_with_defs(schema: &Value, defs: Option<&Value>) -> String {
    match schema {
        Value::Object(obj) => {
            // Handle $ref
            if let Some(ref_val) = obj.get("$ref").and_then(|v| v.as_str()) {
                // Extract definition name from "#/$defs/TypeName" or "#/definitions/TypeName"
                let def_name = ref_val
                    .strip_prefix("#/$defs/")
                    .or_else(|| ref_val.strip_prefix("#/definitions/"));

                if let (Some(name), Some(defs_val)) = (def_name, defs)
                    && let Some(def) = defs_val.get(name)
                {
                    return json_schema_to_typescript_with_defs(def, defs);
                }
                return "unknown".to_string();
            }

            if let Some(one_of) = obj.get("oneOf").and_then(|v| v.as_array()) {
                let types: Vec<String> = one_of
                    .iter()
                    .map(|v| json_schema_to_typescript_with_defs(v, defs))
                    .collect();
                return types.join(" | ");
            }

            if let Some(any_of) = obj.get("anyOf").and_then(|v| v.as_array()) {
                let types: Vec<String> = any_of
                    .iter()
                    .map(|v| json_schema_to_typescript_with_defs(v, defs))
                    .collect();
                return types.join(" | ");
            }

            if let Some(type_val) = obj.get("type") {
                match type_val.as_str() {
                    Some("string") => "string".to_string(),
                    Some("number") | Some("integer") => "number".to_string(),
                    Some("boolean") => "boolean".to_string(),
                    Some("null") => "null".to_string(),
                    Some("array") => {
                        let items_type = obj
                            .get("items")
                            .map(|v| json_schema_to_typescript_with_defs(v, defs))
                            .unwrap_or_else(|| "unknown".to_string());
                        format!("{items_type}[]")
                    }
                    Some("object") => {
                        if let Some(props) = obj.get("properties").and_then(|p| p.as_object()) {
                            let required = obj
                                .get("required")
                                .and_then(|r| r.as_array())
                                .map(|arr| {
                                    arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()
                                })
                                .unwrap_or_default();

                            let fields: Vec<String> = props
                                .iter()
                                .map(|(k, v)| {
                                    let ts_type = json_schema_to_typescript_with_defs(v, defs);
                                    let optional = if required.contains(&k.as_str()) {
                                        ""
                                    } else {
                                        "?"
                                    };
                                    format!("{k}{optional}: {ts_type}")
                                })
                                .collect();
                            format!("{{ {} }}", fields.join("; "))
                        } else {
                            "Record<string, unknown>".to_string()
                        }
                    }
                    _ => "unknown".to_string(),
                }
            } else {
                "unknown".to_string()
            }
        }
        _ => "unknown".to_string(),
    }
}

fn to_pascal_case(s: &str) -> String {
    s.split(['_', '-'])
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::Tool;
    use serde_json::json;
    use std::sync::Arc;

    fn make_tool(name: &str, description: &str, schema: Value) -> Tool {
        let name = name.to_string();
        let description = description.to_string();
        Tool {
            name: name.into(),
            description: Some(description.into()),
            input_schema: Arc::new(schema.as_object().cloned().unwrap_or_default()),
            title: None,
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
        }
    }

    #[test]
    fn test_simple_tool_generation() {
        let tool = make_tool(
            "get_items",
            "Get all items",
            json!({
                "type": "object",
                "properties": {}
            }),
        );

        let ts = generate_typescript_interface(&[tool], "tools");
        assert!(ts.contains("declare namespace tools"));
        assert!(ts.contains("function get_items(): unknown"));
    }

    #[test]
    fn test_tool_with_params() {
        let tool = make_tool(
            "move_footprint",
            "Move a footprint to a new position",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "UUID of the footprint"
                    },
                    "x_mm": {
                        "type": "number",
                        "description": "X position in mm"
                    },
                    "y_mm": {
                        "type": "number",
                        "description": "Y position in mm"
                    },
                    "rotation": {
                        "type": "number",
                        "description": "Optional rotation"
                    }
                },
                "required": ["id", "x_mm", "y_mm"]
            }),
        );

        let ts = generate_typescript_interface(&[tool], "kicad");

        assert!(ts.contains("declare namespace kicad"));
        assert!(ts.contains("interface MoveFootprintParams"));
        assert!(ts.contains("id: string"));
        assert!(ts.contains("x_mm: number"));
        assert!(ts.contains("rotation?: number")); // optional
        assert!(ts.contains("function move_footprint(params: MoveFootprintParams): unknown"));
    }

    #[test]
    fn test_array_type() {
        let tool = make_tool(
            "get_items_by_id",
            "Get items by IDs",
            json!({
                "type": "object",
                "properties": {
                    "item_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of item UUIDs"
                    }
                },
                "required": ["item_ids"]
            }),
        );

        let ts = generate_typescript_interface(&[tool], "tools");
        assert!(ts.contains("item_ids: string[]"));
    }

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("get_items"), "GetItems");
        assert_eq!(to_pascal_case("move-footprint"), "MoveFootprint");
        assert_eq!(to_pascal_case("simple"), "Simple");
    }

    #[test]
    fn test_json_schema_to_typescript() {
        assert_eq!(
            json_schema_to_typescript(&json!({"type": "string"})),
            "string"
        );
        assert_eq!(
            json_schema_to_typescript(&json!({"type": "number"})),
            "number"
        );
        assert_eq!(
            json_schema_to_typescript(&json!({"type": "integer"})),
            "number"
        );
        assert_eq!(
            json_schema_to_typescript(&json!({"type": "boolean"})),
            "boolean"
        );
        assert_eq!(
            json_schema_to_typescript(&json!({"type": "array", "items": {"type": "string"}})),
            "string[]"
        );
    }

    #[test]
    fn test_nullable_type() {
        let ts = json_schema_to_typescript(&json!({
            "anyOf": [
                {"type": "string"},
                {"type": "null"}
            ]
        }));
        assert!(ts.contains("string"));
        assert!(ts.contains("null"));
    }
}
