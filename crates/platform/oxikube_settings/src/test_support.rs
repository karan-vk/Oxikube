//! Setting types shared by the unit tests.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;

/// Defaults used by the store tests (JSONC on purpose).
pub const DEFAULTS: &str = r#"
// test defaults
{
  "ui_scale": 1.0,
  "read_only": false,
  "terminal": {
    "font_size": 12,
    "shell": "/bin/sh",
    "env": {},
  },
  "clusters": {},
}
"#;

/// Content of the keyed `terminal` section.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct TerminalContent {
    /// Font size in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    /// Line height as a multiple of the font size (an `f64` field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f64>,
    /// Shell to start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Extra environment (an open map).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    /// Shell arguments (an array).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
}

/// Resolved `terminal` settings.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalSettings {
    pub font_size: f32,
    pub line_height: f64,
    pub shell: String,
    pub env: BTreeMap<String, String>,
    pub args: Vec<String>,
}

impl Settings for TerminalSettings {
    const KEY: Option<&'static str> = Some("terminal");
    type Content = TerminalContent;

    fn from_content(content: TerminalContent) -> Self {
        Self {
            font_size: content.font_size.unwrap_or_default(),
            line_height: content.line_height.unwrap_or_default(),
            shell: content.shell.unwrap_or_default(),
            env: content.env.unwrap_or_default(),
            args: content.args.unwrap_or_default(),
        }
    }
}

/// Root-level content (`KEY = None`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct GeneralContent {
    /// UI zoom factor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_scale: Option<f32>,
    /// Refuse mutations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
}

/// Resolved root-level settings.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneralSettings {
    pub ui_scale: f32,
    pub read_only: bool,
}

impl Settings for GeneralSettings {
    const KEY: Option<&'static str> = None;
    type Content = GeneralContent;

    fn from_content(content: GeneralContent) -> Self {
        Self {
            ui_scale: content.ui_scale.unwrap_or_default(),
            read_only: content.read_only.unwrap_or_default(),
        }
    }
}

/// A store over [`DEFAULTS`] with both test settings registered.
pub fn store() -> crate::SettingsStore {
    let mut store = crate::SettingsStore::without_registered(DEFAULTS).unwrap();
    store.register_setting::<TerminalSettings>();
    store.register_setting::<GeneralSettings>();
    store
}
