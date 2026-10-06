//! Reading one node of an OpenAPI v3 schema: how it is typed, described and opened.

use serde_json::Value;

/// How many characters of a description a row keeps (the first paragraph, cut at a word).
pub(super) const MAX_DESCRIPTION: usize = 280;
/// How many enum values a row keeps.
pub(super) const MAX_ENUM: usize = 24;

fn str_of<'a>(node: &'a Value, key: &str) -> Option<&'a str> {
    node.get(key).and_then(Value::as_str)
}

fn flag(node: &Value, key: &str) -> bool {
    node.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// `node`'s object-valued `additionalProperties` schema (a map's values), if it has one.
fn values_of(node: &Value) -> Option<&Value> {
    node.get("additionalProperties").filter(|v| v.is_object())
}

/// The type as `kubectl explain` writes it.
pub(super) fn type_text(node: &Value) -> String {
    if flag(node, "x-kubernetes-int-or-string") {
        return "int-or-string".to_owned();
    }
    match str_of(node, "type") {
        Some("array") => match node.get("items") {
            Some(items) => format!("[]{}", type_text(items)),
            None => "[]any".to_owned(),
        },
        Some("object") => match values_of(node) {
            Some(values) if node.get("properties").is_none() => {
                format!("map[string]{}", type_text(values))
            }
            _ if node.get("properties").is_none()
                && flag(node, "x-kubernetes-preserve-unknown-fields") =>
            {
                "object (free-form)".to_owned()
            }
            _ => "object".to_owned(),
        },
        Some(ty) => match str_of(node, "format") {
            Some(format) if !format.is_empty() => format!("{ty} ({format})"),
            _ => ty.to_owned(),
        },
        None if flag(node, "x-kubernetes-preserve-unknown-fields") => "any".to_owned(),
        None => String::new(),
    }
}

/// The first paragraph of the description with whitespace collapsed, cut at
/// [`MAX_DESCRIPTION`] characters on a word boundary.
pub(super) fn description(node: &Value) -> Option<String> {
    let text = str_of(node, "description")?;
    let first = text.split("\n\n").next().unwrap_or(text);
    let collapsed = first.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    if collapsed.chars().count() <= MAX_DESCRIPTION {
        return Some(collapsed);
    }
    let cut: String = collapsed.chars().take(MAX_DESCRIPTION).collect();
    let at = cut.rfind(' ').unwrap_or(cut.len());
    Some(format!(
        "{}…",
        cut[..at].trim_end_matches([',', ';', ':', '.'])
    ))
}

/// The enum values as text (strings as they are, others as JSON), up to [`MAX_ENUM`].
pub(super) fn enum_values(node: &Value) -> Vec<String> {
    node.get("enum")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .take(MAX_ENUM)
                .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The default as text, when the schema has one.
pub(super) fn default_text(node: &Value) -> Option<String> {
    let value = node.get("default")?;
    let text = value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned);
    Some(if text.chars().count() > 60 {
        format!("{}…", text.chars().take(60).collect::<String>())
    } else {
        text
    })
}

/// The node whose `properties` `node` opens into: itself, or the element or value schema of an
/// array or a map of them (looking through nesting), whichever has properties. `None` when
/// there is nothing to open.
pub(super) fn container(node: &Value) -> Option<&Value> {
    let mut current = node;
    // A schema cannot nest deeper than the JSON it is in; the bound is only a guard.
    for _ in 0..64 {
        if current
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|p| !p.is_empty())
        {
            return Some(current);
        }
        current = match (current.get("items"), values_of(current)) {
            (Some(items), _) if items.is_object() => items,
            (_, Some(values)) => values,
            _ => return None,
        };
    }
    None
}

/// The fields of the object `node` opens into: required ones first, then by name.
pub(super) fn fields(node: &Value) -> Vec<(&str, &Value, bool)> {
    let Some(parent) = container(node) else {
        return Vec::new();
    };
    let required: Vec<&str> = parent
        .get("required")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut fields: Vec<(&str, &Value, bool)> = parent
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| {
            props
                .iter()
                .map(|(name, schema)| (name.as_str(), schema, required.contains(&name.as_str())))
                .collect()
        })
        .unwrap_or_default();
    fields.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(b.0)));
    fields
}
