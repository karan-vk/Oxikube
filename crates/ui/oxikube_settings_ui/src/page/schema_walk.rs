//! Reading the settings schema: following `$ref`s and nullable wrappers to the shape of a field.
//!
//! The schema is `SettingsStore::json_schema()`: draft 2019-09 from `schemars`, with every
//! struct a `#/$defs/<Name>` reference and every optional field either `"type": [.., "null"]` or
//! `"anyOf": [<schema>, {"type": "null"}]`.

use gpui::SharedString;
use serde_json::{Map, Value};

use super::model::FieldKind;

/// The schema's `$defs`, for resolving references.
pub(crate) struct Definitions<'a>(Option<&'a Map<String, Value>>);

impl<'a> Definitions<'a> {
    /// The definitions of `schema` (none when it has no `$defs`).
    pub fn of(schema: &'a Value) -> Self {
        Self(schema.get("$defs").and_then(Value::as_object))
    }

    /// `node` with its `$ref` and a nullable `anyOf` wrapper followed, until a node that is
    /// neither. A reference that does not resolve stops the walk at the reference.
    pub fn resolve(&self, node: &'a Value) -> &'a Value {
        let mut current = node;
        // A bound on the walk: a reference cycle in a broken schema must not hang the page.
        for _ in 0..16 {
            if let Some(target) = current
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
                .and_then(|name| self.0?.get(name))
            {
                current = target;
                continue;
            }
            match non_null_branch(current) {
                Some(branch) => current = branch,
                None => return current,
            }
        }
        current
    }

    /// What kind of value `node` holds.
    pub fn kind_of(&self, node: &'a Value) -> FieldKind {
        let resolved = self.resolve(node);
        if let Some(choices) = choices(resolved) {
            return FieldKind::Choice(choices);
        }
        if resolved.get("anyOf").is_some() || resolved.get("oneOf").is_some() {
            return FieldKind::Other;
        }
        match primary_type(resolved) {
            Some("boolean") => FieldKind::Toggle,
            Some("integer") => FieldKind::Integer,
            Some("number") => FieldKind::Number,
            Some("string") => FieldKind::Text,
            Some("array") => FieldKind::List,
            Some("object") => FieldKind::Map,
            _ => FieldKind::Other,
        }
    }

    /// The description of `node`: its own, else that of what it resolves to.
    pub fn description_of(&self, node: &'a Value) -> Option<&'a str> {
        description(node).or_else(|| description(self.resolve(node)))
    }
}

/// The one branch of an `anyOf` that is not `{"type": "null"}`, when there is exactly one.
fn non_null_branch(node: &Value) -> Option<&Value> {
    let branches = node.get("anyOf")?.as_array()?;
    let mut others = branches
        .iter()
        .filter(|branch| branch.get("type") != Some(&Value::from("null")));
    let first = others.next()?;
    others.next().is_none().then_some(first)
}

/// The non-null `type` of `node`: `"string"` for `"string"` and for `["string", "null"]`.
fn primary_type(node: &Value) -> Option<&str> {
    match node.get("type")? {
        Value::String(name) => Some(name),
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .find(|name| *name != "null"),
        _ => None,
    }
}

/// The values of an enumeration: a plain `enum` of strings, or `schemars`' `oneOf` of `const`s
/// (an enum whose variants have doc comments).
fn choices(node: &Value) -> Option<Vec<SharedString>> {
    if let Some(values) = node.get("enum").and_then(Value::as_array) {
        let strings: Vec<SharedString> = values
            .iter()
            .filter_map(Value::as_str)
            .map(|value| SharedString::from(value.to_owned()))
            .collect();
        return (!strings.is_empty()).then_some(strings);
    }
    let branches = node.get("oneOf")?.as_array()?;
    branches
        .iter()
        .map(|branch| {
            branch
                .get("const")
                .and_then(Value::as_str)
                .map(|value| SharedString::from(value.to_owned()))
        })
        .collect()
}

/// `node`'s `description`, trimmed; `None` when blank.
fn description(node: &Value) -> Option<&str> {
    node.get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}
