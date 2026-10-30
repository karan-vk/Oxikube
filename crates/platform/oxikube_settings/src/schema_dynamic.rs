//! Runtime enum injection for the settings schema (E21-S02).
//!
//! The checked-in `settings.schema.json` is static and deterministic, so it cannot know the
//! themes a user dropped into their themes directory, the fonts installed on this machine or the
//! base keymaps the running build ships. Fields that want such a list carry an annotation
//! ([`DYNAMIC_ENUM_KEY`], declared once here) naming the list;
//! [`SettingsStore::json_schema_with`](crate::SettingsStore::json_schema_with) overlays the live
//! values as an `enum` next to the annotation. Platform crates stay independent of each other:
//! the binary supplies the lists.

use std::collections::BTreeMap;

use serde_json::Value;

/// The schema annotation naming the live list a string field draws its values from.
pub const DYNAMIC_ENUM_KEY: &str = "x-oxikube-dynamic-enum";

/// List name for installed theme names.
pub const THEMES: &str = "themes";
/// List name for the base keymap names.
pub const BASE_KEYMAPS: &str = "base_keymaps";
/// List name for the installed font families.
pub const FONTS: &str = "fonts";

/// The live lists to overlay on the schema, keyed by list name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DynamicEnums {
    lists: BTreeMap<String, Vec<String>>,
}

impl DynamicEnums {
    /// No lists: [`apply`](Self::apply) leaves the schema unchanged.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the values of the list `name` (duplicates are dropped, order is kept).
    #[must_use]
    pub fn with(mut self, name: &str, values: impl IntoIterator<Item = String>) -> Self {
        let mut unique: Vec<String> = Vec::new();
        for value in values {
            if !unique.contains(&value) {
                unique.push(value);
            }
        }
        self.lists.insert(name.to_owned(), unique);
        self
    }

    /// The values of the list `name`, if supplied.
    pub fn list(&self, name: &str) -> Option<&[String]> {
        self.lists.get(name).map(Vec::as_slice)
    }

    /// Add `enum` to every schema node annotated with [`DYNAMIC_ENUM_KEY`] whose list was
    /// supplied. A nullable field (`type: [.., "null"]`, or an `anyOf` with a null branch) keeps
    /// accepting `null`. Nodes whose list is empty or unknown are left alone.
    pub fn apply(&self, schema: &mut Value) {
        match schema {
            Value::Object(map) => {
                let values = map
                    .get(DYNAMIC_ENUM_KEY)
                    .and_then(Value::as_str)
                    .and_then(|name| self.list(name))
                    .filter(|values| !values.is_empty())
                    .map(<[String]>::to_vec);
                if let Some(values) = values {
                    let mut allowed: Vec<Value> = values.into_iter().map(Value::String).collect();
                    if accepts_null(map) {
                        allowed.push(Value::Null);
                    }
                    map.insert("enum".to_owned(), Value::Array(allowed));
                }
                map.values_mut().for_each(|sub| self.apply(sub));
            }
            Value::Array(items) => items.iter_mut().for_each(|sub| self.apply(sub)),
            _ => {}
        }
    }
}

fn accepts_null(node: &serde_json::Map<String, Value>) -> bool {
    let in_type = node
        .get("type")
        .and_then(Value::as_array)
        .is_some_and(|types| types.iter().any(|ty| ty == "null"));
    let in_any_of = node
        .get("anyOf")
        .and_then(Value::as_array)
        .is_some_and(|branches| {
            branches
                .iter()
                .any(|b| b.get("type") == Some(&Value::from("null")))
        });
    in_type || in_any_of
}

#[cfg(test)]
mod tests;
