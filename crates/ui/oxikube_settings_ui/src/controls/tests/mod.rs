//! Tests of the controls: the control chosen per schema type, validation, and the page writing,
//! resetting and rejecting through a real settings store over a temporary directory.

mod page;
mod spec;
mod validate;

use serde_json::{Value, json};

use crate::page::SettingsPageModel;

/// A schema with one field of each control kind.
pub(super) fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "confirm_quit": { "type": ["boolean", "null"], "description": "Ask before quitting." },
            "terminal": { "$ref": "#/$defs/Terminal" },
            "editor": { "$ref": "#/$defs/Editor" }
        },
        "$defs": {
            "Terminal": {
                "type": "object",
                "properties": {
                    "font_size": { "type": ["number", "null"], "minimum": 6.0, "maximum": 72.0 },
                    "scrollback_lines": { "type": ["integer", "null"], "minimum": 0 },
                    "cursor_shape": { "anyOf": [{ "$ref": "#/$defs/Shape" }, { "type": "null" }] },
                    "exec_shells": { "type": ["array", "null"], "items": { "type": "string" } },
                    "env": { "type": ["object", "null"], "additionalProperties": { "type": "string" } },
                    "accent": { "type": ["string", "null"], "format": "color" },
                    "shell": { "type": ["string", "null"] }
                }
            },
            "Shape": {
                "oneOf": [
                    { "const": "block", "type": "string" },
                    { "const": "bar", "type": "string" }
                ]
            },
            "Editor": {
                "type": "object",
                "properties": {
                    "font_family": { "type": ["string", "null"], "x-oxikube-dynamic-enum": "fonts" },
                    "colour": {
                        "anyOf": [
                            { "$ref": "#/$defs/Hex" },
                            { "enum": ["red", "green"] },
                            { "type": "null" }
                        ]
                    }
                }
            },
            "Hex": { "type": "string", "pattern": "^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$" }
        }
    })
}

pub(super) fn model() -> SettingsPageModel {
    SettingsPageModel::from_schema(&schema())
}
