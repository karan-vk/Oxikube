//! How a schema node is described to the reader: its type, its allowed values and its docs.

use oxikube_domain::schema::{AdditionalProperties, JsonSchema};

/// The type as the hover and the menu name it: `integer`, `string (enum)`, `array of object`,
/// `map of string`, `int or string`.
pub(super) fn type_label(schema: &JsonSchema) -> String {
    let base = base_label(schema);
    if schema.enum_values.is_empty() {
        base
    } else {
        format!("{base} (enum)")
    }
}

fn base_label(schema: &JsonSchema) -> String {
    if schema.xk8s.int_or_string {
        return "int or string".to_owned();
    }
    let types: Vec<&str> = schema.types.iter().map(|t| t.as_str()).collect();
    match types.as_slice() {
        ["array"] | [] if schema.items.is_some() => {
            let items = schema.items.as_deref().map_or_else(String::new, base_label);
            format!("array of {items}")
        }
        ["object"] | [] => match &schema.additional_properties {
            AdditionalProperties::Schema(each) if schema.properties.is_empty() => {
                format!("map of {}", base_label(each))
            }
            _ if types.is_empty() && schema.properties.is_empty() => "any".to_owned(),
            _ => "object".to_owned(),
        },
        [one] => (*one).to_owned(),
        many => many.join(" | "),
    }
}

/// An enum member as the reader and the buffer write it: strings bare, everything else as JSON.
pub(super) fn enum_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `text` as a YAML scalar a manifest can hold: bare when that reads back as the same string,
/// double-quoted otherwise (empty, `yes`, `null`, `1.5`, anything with a YAML indicator in it).
pub(super) fn yaml_scalar(text: &str) -> String {
    let bare = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
        && !text.starts_with(['-', '.'])
        && !looks_typed(text);
    if bare {
        text.to_owned()
    } else {
        serde_json::Value::String(text.to_owned()).to_string()
    }
}

/// Whether a bare `text` would be read as a bool, null or number rather than a string.
fn looks_typed(text: &str) -> bool {
    const WORDS: [&str; 10] = [
        "true", "false", "yes", "no", "on", "off", "null", "y", "n", "~",
    ];
    WORDS.iter().any(|w| text.eq_ignore_ascii_case(w))
        || text.parse::<f64>().is_ok()
        || text.starts_with("0x")
        || text.starts_with("0o")
}

/// The Markdown note of a field for the completion menu: its description, then its values.
pub(super) fn field_docs(schema: &JsonSchema) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(description) = schema.description.as_deref().filter(|d| !d.is_empty()) {
        parts.push(description.to_owned());
    }
    if !schema.enum_values.is_empty() {
        parts.push(format!(
            "Allowed values: {}",
            code_list(schema.enum_values.iter().map(enum_text))
        ));
    }
    if let Some(default) = &schema.default {
        parts.push(format!("Default: `{}`", enum_text(default)));
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// `` `a`, `b`, `c` ``.
pub(super) fn code_list(values: impl IntoIterator<Item = String>) -> String {
    values
        .into_iter()
        .map(|v| format!("`{v}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema(node: serde_json::Value) -> JsonSchema {
        JsonSchema::from_value(&node)
    }

    #[test]
    fn names_the_type_of_a_node() {
        let label = |node| type_label(&schema(node));
        assert_eq!(label(json!({"type": "integer"})), "integer");
        assert_eq!(
            label(json!({"type": "string", "enum": ["Always", "Never"]})),
            "string (enum)"
        );
        assert_eq!(
            label(json!({"type": "array", "items": {"type": "object", "properties": {"a": {}}}})),
            "array of object"
        );
        assert_eq!(
            label(json!({"type": "array", "items": {"type": "string"}})),
            "array of string"
        );
        assert_eq!(
            label(json!({"type": "object", "additionalProperties": {"type": "string"}})),
            "map of string"
        );
        assert_eq!(
            label(json!({"x-kubernetes-int-or-string": true})),
            "int or string"
        );
        assert_eq!(
            label(json!({"type": "object", "properties": {"a": {}}})),
            "object"
        );
        assert_eq!(label(json!({})), "any");
    }

    #[test]
    fn quotes_only_what_would_read_back_differently() {
        assert_eq!(yaml_scalar("Always"), "Always");
        assert_eq!(
            yaml_scalar("ClusterFirstWithHostNet"),
            "ClusterFirstWithHostNet"
        );
        assert_eq!(yaml_scalar("true"), "\"true\"");
        assert_eq!(yaml_scalar("yes"), "\"yes\"");
        assert_eq!(yaml_scalar("1.5"), "\"1.5\"");
        assert_eq!(yaml_scalar(""), "\"\"");
        assert_eq!(yaml_scalar("a b"), "\"a b\"");
        assert_eq!(yaml_scalar("a: b"), "\"a: b\"");
    }

    #[test]
    fn docs_list_the_values_and_the_default() {
        let node = schema(json!({
            "type": "string", "description": "Protocol.", "enum": ["TCP", "UDP"], "default": "TCP"
        }));
        let docs = field_docs(&node).unwrap();
        assert!(docs.starts_with("Protocol."));
        assert!(docs.contains("Allowed values: `TCP`, `UDP`"));
        assert!(docs.contains("Default: `TCP`"));
        assert_eq!(field_docs(&schema(json!({"type": "string"}))), None);
    }
}
