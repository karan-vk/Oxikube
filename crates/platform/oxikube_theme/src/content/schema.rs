//! The JSON Schema of a theme-family file, built from the importer's key table so the two cannot
//! drift.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde_json::{Map, Value, json};

use super::{OxikubeContent, ThemeFamilyContent, ThemeStyleContent};
use crate::import::table;

/// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa` (what [`crate::color::parse_color`] accepts).
const COLOR_PATTERN: &str = "^#([0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$";

fn color_schema() -> Value {
    json!({ "type": ["string", "null"], "pattern": COLOR_PATTERN })
}

impl JsonSchema for ThemeStyleContent {
    fn schema_name() -> Cow<'static, str> {
        "ThemeStyleContent".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let mut properties = Map::new();
        let mut keys: Vec<&str> = table::mapped_keys().collect();
        keys.sort_unstable();
        for key in keys {
            properties.insert(key.to_owned(), color_schema());
        }
        properties.insert(
            "players".to_owned(),
            json!({ "type": "array", "items": { "type": "object", "properties": {
                "cursor": color_schema(),
                "background": color_schema(),
                "selection": color_schema(),
            } } }),
        );
        properties.insert(
            "accents".to_owned(),
            json!({ "type": "array", "items": color_schema() }),
        );
        properties.insert(
            "syntax".to_owned(),
            json!({ "type": "object", "additionalProperties": { "type": "object", "properties": {
                "color": color_schema(),
                "font_style": { "type": ["string", "null"], "enum": ["normal", "italic", "oblique", null] },
                "font_weight": { "type": ["integer", "null"] },
            } } }),
        );
        json_schema!({
            "type": "object",
            "description": "Theme colours by dotted Zed key; unknown keys are ignored (and counted as unmapped).",
            "properties": properties,
        })
    }
}

impl JsonSchema for OxikubeContent {
    fn schema_name() -> Cow<'static, str> {
        "OxikubeContent".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let mut properties = Map::new();
        let mut keys = table::oxikube_keys();
        keys.sort_unstable();
        for key in keys {
            properties.insert(key, color_schema());
        }
        json_schema!({
            "type": "object",
            "description": "Oxikube's Kubernetes status colours; unset keys are derived from `style`.",
            "properties": properties,
        })
    }
}

/// The schema of a theme-family file.
#[must_use]
pub fn theme_family_schema() -> Schema {
    schemars::generate::SchemaSettings::draft2019_09()
        .into_generator()
        .into_root_schema_for::<ThemeFamilyContent>()
}

/// [`theme_family_schema`] as canonical pretty JSON (keys sorted, trailing newline), the text
/// committed as `schema/theme-family.schema.json`.
///
/// # Panics
///
/// Never in practice: a schema is always serialisable.
#[must_use]
pub fn theme_family_schema_text() -> String {
    let value = serde_json::to_value(theme_family_schema()).expect("a schema serialises");
    let mut text = serde_json::to_string_pretty(&sorted(value)).expect("a value serialises");
    text.push('\n');
    text
}

/// Recursively sorts object keys so the output does not depend on map order.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}
