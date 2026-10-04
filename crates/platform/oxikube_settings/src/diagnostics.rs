//! Problems found while loading a settings layer, and unknown-key detection.
//!
//! Loading never fails as a whole: a syntax error keeps the last good layer, a type error
//! keeps the affected setting's last good value, and an unknown key is ignored. Each case is
//! recorded as a [`SettingsDiagnostic`] for the UI to surface (a toast, later the settings
//! page).

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Map, Value};

/// Key reserved for the per-cluster override layer.
pub const CLUSTERS_KEY: &str = "clusters";
/// Key editors use to point at a schema; always allowed at the root.
pub const SCHEMA_KEY: &str = "$schema";

/// One problem found while loading settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsDiagnostic {
    /// The user's `settings.json` is not valid JSONC or not an object. The last good user
    /// settings stay in effect.
    InvalidJson {
        /// Parser message with line and column.
        message: String,
    },
    /// A key that no registered setting reads (a typo, or a setting from a newer version).
    UnknownKey {
        /// Dotted path from the root, e.g. `clusters.<id>.terminal.font`.
        path: String,
    },
    /// A value of the wrong type. The setting keeps its last good value.
    InvalidValue {
        /// Rust type name of the setting.
        setting: &'static str,
        /// The cluster layer the value was read for, `None` for the global value.
        cluster: Option<String>,
        /// Deserialiser message including the field path.
        message: String,
    },
}

impl fmt::Display for SettingsDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson { message } => write!(f, "settings.json is invalid: {message}"),
            Self::UnknownKey { path } => write!(f, "unknown setting `{path}`"),
            Self::InvalidValue {
                setting,
                cluster: None,
                message,
            } => write!(f, "invalid value for {setting}: {message}"),
            Self::InvalidValue {
                setting,
                cluster: Some(cluster),
                message,
            } => write!(
                f,
                "invalid value for {setting} in cluster {cluster}: {message}"
            ),
        }
    }
}

/// The keys a setting's content accepts, derived from its JSON schema.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyTree {
    /// Anything goes below this point (scalars, maps, arrays, recursive types).
    Any,
    /// An object with a fixed set of keys.
    Object(BTreeMap<String, KeyTree>),
}

impl KeyTree {
    /// Build the tree from a schema generated with inlined subschemas.
    pub fn from_schema(schema: &Value) -> KeyTree {
        let Some(schema) = schema.as_object() else {
            return KeyTree::Any;
        };
        if let Some(Value::Object(properties)) = schema.get("properties") {
            let open = matches!(
                schema.get("additionalProperties"),
                Some(Value::Object(_) | Value::Bool(true))
            );
            if open {
                return KeyTree::Any;
            }
            return KeyTree::Object(
                properties
                    .iter()
                    .map(|(key, sub)| (key.clone(), KeyTree::from_schema(sub)))
                    .collect(),
            );
        }
        // `Option<Struct>` and untagged enums: accept the union of the object variants.
        for combinator in ["anyOf", "oneOf", "allOf"] {
            if let Some(Value::Array(variants)) = schema.get(combinator) {
                let mut keys = BTreeMap::new();
                for variant in variants {
                    match KeyTree::from_schema(variant) {
                        KeyTree::Object(sub) => keys.extend(sub),
                        KeyTree::Any if is_object_like(variant) => return KeyTree::Any,
                        KeyTree::Any => {}
                    }
                }
                return if keys.is_empty() {
                    KeyTree::Any
                } else {
                    KeyTree::Object(keys)
                };
            }
        }
        KeyTree::Any
    }
}

/// Whether a schema might describe an object we cannot enumerate (a map or a `$ref`).
fn is_object_like(schema: &Value) -> bool {
    let Some(schema) = schema.as_object() else {
        return true;
    };
    schema.contains_key("$ref")
        || schema.contains_key("additionalProperties")
        || schema.get("type") == Some(&Value::String("object".into()))
}

/// Append the dotted path of every key in `value` that `keys` does not know.
pub fn collect_unknown_keys(
    value: &Map<String, Value>,
    keys: &BTreeMap<String, KeyTree>,
    prefix: &str,
    out: &mut Vec<SettingsDiagnostic>,
) {
    for (key, sub_value) in value {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match keys.get(key) {
            None => out.push(SettingsDiagnostic::UnknownKey { path }),
            Some(KeyTree::Object(sub_keys)) => {
                if let Value::Object(sub_map) = sub_value {
                    collect_unknown_keys(sub_map, sub_keys, &path, out);
                }
            }
            Some(KeyTree::Any) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tree(schema: Value) -> KeyTree {
        KeyTree::from_schema(&schema)
    }

    #[test]
    fn struct_schema_lists_its_keys_and_maps_are_open() {
        let schema = json!({
            "type": "object",
            "properties": {
                "font": {"type": ["string", "null"]},
                "nested": {"anyOf": [
                    {"type": "object", "properties": {"x": {"type": "integer"}}},
                    {"type": "null"}
                ]},
                "env": {"type": "object", "additionalProperties": {"type": "string"}},
            }
        });
        let KeyTree::Object(keys) = tree(schema) else {
            panic!("expected an object tree");
        };
        assert_eq!(keys["font"], KeyTree::Any);
        assert_eq!(keys["env"], KeyTree::Any);
        assert!(matches!(&keys["nested"], KeyTree::Object(n) if n.contains_key("x")));
    }

    #[test]
    fn unknown_keys_are_reported_with_their_path() {
        let KeyTree::Object(keys) = tree(json!({
            "properties": {
                "terminal": {"properties": {"font": {}}},
                "env": {"additionalProperties": {}},
            }
        })) else {
            panic!("expected an object tree");
        };
        let value = json!({
            "terminal": {"font": "x", "fnot": 1},
            "env": {"ANY": "thing"},
            "typo": true,
        });
        let mut out = Vec::new();
        collect_unknown_keys(value.as_object().unwrap(), &keys, "", &mut out);
        assert_eq!(
            out,
            vec![
                SettingsDiagnostic::UnknownKey {
                    path: "terminal.fnot".into()
                },
                SettingsDiagnostic::UnknownKey {
                    path: "typo".into()
                },
            ]
        );
    }
}
