//! A fixed settings schema for the page's tests and screenshot: the shapes the real schema has
//! (a `$ref` section, a nullable `anyOf` section, `oneOf` enums, nullable scalars, a union),
//! with paths that exist in the embedded `default.json`, so the badges and values read real
//! layers. The real schema changes with every registered setting; the tests must not.

use std::path::Path;

use gpui::App;
use serde_json::{Value, json};

use super::SettingsPageModel;

/// The fixture schema: the shapes of the real one, with paths `default.json` has.
pub fn fixture_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "$schema": { "type": "string", "description": "Schema reference for editors." },
            "clusters": { "type": "object", "description": "Per-cluster overrides." },
            "ui_scale": {
                "type": ["number", "null"],
                "description": "UI zoom factor: 1.0 is 100 %. Values outside 0.5 to 3.0 are clamped."
            },
            "confirm_quit": {
                "type": ["boolean", "null"],
                "description": "Ask before quitting while exec sessions or port-forwards are running."
            },
            "base_keymap": {
                "anyOf": [{ "$ref": "#/$defs/BaseKeymap" }, { "type": "null" }],
                "description": "The keymap layered under your keymap.json."
            },
            "theme": { "$ref": "#/$defs/ThemeSelectionContent" },
            "terminal": { "$ref": "#/$defs/TerminalContent" },
            "editor": {
                "anyOf": [{ "$ref": "#/$defs/EditorSettingsContent" }, { "type": "null" }]
            }
        },
        "$defs": {
            "BaseKeymap": {
                "description": "The base keymap a user chooses.",
                "oneOf": [
                    { "const": "default", "type": "string", "description": "The per-OS defaults only." },
                    { "const": "vim", "type": "string", "description": "The defaults plus the vim layer." }
                ]
            },
            "ThemeSelectionContent": {
                "description": "What one settings layer says about the theme.",
                "anyOf": [
                    { "type": "string", "description": "One theme." },
                    { "type": "object", "properties": { "mode": { "type": ["string", "null"] } } }
                ]
            },
            "TerminalContent": {
                "description": "What one settings layer says about terminals.",
                "type": "object",
                "properties": {
                    "font_size": {
                        "type": ["number", "null"],
                        "description": "Font size of terminals, in points."
                    },
                    "cursor_blink": {
                        "type": ["boolean", "null"],
                        "description": "Whether the cursor blinks (default false)."
                    },
                    "cursor_shape": {
                        "anyOf": [{ "$ref": "#/$defs/CursorShapeSetting" }, { "type": "null" }],
                        "description": "The cursor shape until the process sets its own."
                    },
                    "copy_on_select": {
                        "type": ["boolean", "null"],
                        "description": "Copy the selection to the clipboard as soon as the mouse button is released."
                    },
                    "scrollback_lines": {
                        "type": ["integer", "null"],
                        "description": "How many lines of output a terminal keeps; never written to disk."
                    },
                    "exec_shells": {
                        "type": ["array", "null"],
                        "items": { "type": "string" },
                        "description": "The shells pod::Shell tries in a container, first to last."
                    },
                    "env": {
                        "type": ["object", "null"],
                        "additionalProperties": { "type": "string" },
                        "description": "Extra environment variables for local shells."
                    }
                }
            },
            "CursorShapeSetting": {
                "oneOf": [
                    { "const": "block", "type": "string" },
                    { "const": "bar", "type": "string" },
                    { "const": "underline", "type": "string" }
                ]
            },
            "EditorSettingsContent": {
                "description": "What one settings layer says about the manifest editor.",
                "type": "object",
                "properties": {
                    "tab_size": {
                        "type": ["integer", "null"],
                        "description": "Spaces per indentation level."
                    },
                    "soft_wrap": {
                        "type": ["boolean", "null"],
                        "description": "Wrap long lines at the edge of the editor."
                    },
                    "font_family": {
                        "type": ["string", "null"],
                        "description": "The editor's font family."
                    }
                }
            }
        }
    })
}

/// The page model of [`fixture_schema`].
pub fn fixture_model() -> SettingsPageModel {
    SettingsPageModel::from_schema(&fixture_schema())
}

/// Writes `user_settings` as `settings.json` in `dir` and installs the settings store over it
/// (no watcher), so the page's badges and values read those layers.
///
/// # Panics
///
/// When the file cannot be written: the test's temporary directory is broken.
pub fn install_settings(dir: &Path, user_settings: &str, cx: &mut App) {
    std::fs::write(dir.join("settings.json"), user_settings)
        .expect("the test's config directory is writable");
    oxikube_settings::init_with_dir(dir, cx);
}
