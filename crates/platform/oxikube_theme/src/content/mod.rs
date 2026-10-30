//! Typed serde model of a Zed theme-family file (schema v0.2.0) and its JSON Schema.
//!
//! The importer ([`crate::import`]) deliberately walks a [`serde_json::Value`] through a table so
//! that one bad key never rejects a whole file. This module is the *declarative* description of
//! the same format: it lets tools (editors via `schema/theme-family.schema.json`, extension
//! manifests, tests) read a family as typed data, and it is where the schema comes from. Unknown
//! `style` keys are kept in a map, so a caller can count them as unmapped.
//!
//! Only the file format is shared with Zed; no Zed code is copied.

mod schema;

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use schema::{theme_family_schema, theme_family_schema_text};

/// `appearance` of one theme entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AppearanceContent {
    /// A light theme.
    Light,
    /// A dark theme.
    Dark,
}

impl From<AppearanceContent> for crate::Appearance {
    fn from(content: AppearanceContent) -> Self {
        match content {
            AppearanceContent::Light => Self::Light,
            AppearanceContent::Dark => Self::Dark,
        }
    }
}

/// A whole theme-family file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ThemeFamilyContent {
    /// URL of the schema the file follows (editors use it for completion).
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Family name, e.g. `Ayu`.
    pub name: String,
    /// Author credit.
    #[serde(default)]
    pub author: String,
    /// The themes of the family.
    pub themes: Vec<ThemeContent>,
}

/// One entry of `themes`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ThemeContent {
    /// Theme name, shown in the picker and used by the `theme` setting.
    pub name: String,
    /// Whether the theme is light or dark.
    pub appearance: AppearanceContent,
    /// Colours keyed by dotted Zed key (`editor.background`), plus `players`, `accents` and
    /// `syntax`.
    #[serde(default)]
    pub style: ThemeStyleContent,
    /// Oxikube's own Kubernetes status colours (optional; derived from `style` when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oxikube: Option<OxikubeContent>,
}

/// The `style` map. Mapped colour keys are listed in the emitted schema; any other key is kept
/// (and counted as unmapped by the importer).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ThemeStyleContent(pub BTreeMap<String, Value>);

/// The `oxikube` block: dotted key (`status.running`, `cluster.tab.3`) to a colour string.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OxikubeContent(pub BTreeMap<String, Value>);

#[cfg(test)]
mod tests;
