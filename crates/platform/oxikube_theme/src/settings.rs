//! The `theme` setting: a theme name, or a mode with a light and a dark theme.
//!
//! ```json
//! "theme": "Ayu Dark"
//! "theme": { "mode": "system", "light": "One Light", "dark": "One Dark" }
//! ```
//!
//! Like Zed's. `default.json` sets the second form, so a user may override any subset of it.

use crate::appearance::{Appearance, ThemeMode};
use crate::registry::{DEFAULT_DARK_THEME, DEFAULT_LIGHT_THEME};
use oxikube_settings::Settings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What one settings layer says about the theme.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ThemeSelectionContent {
    /// One theme, whatever the system appearance.
    Static(String),
    /// A light and a dark theme with a mode choosing between them.
    Dynamic {
        /// `system` (follow the OS, the default), `light` or `dark`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<ThemeMode>,
        /// The theme to use in light appearance.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        light: Option<String>,
        /// The theme to use in dark appearance.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dark: Option<String>,
    },
}

impl Default for ThemeSelectionContent {
    fn default() -> Self {
        Self::Dynamic {
            mode: None,
            light: None,
            dark: None,
        }
    }
}

/// The resolved `theme` setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeSelection {
    /// One named theme.
    Static(String),
    /// A mode picking between a light and a dark theme.
    Dynamic {
        /// How to choose.
        mode: ThemeMode,
        /// Theme for light appearance.
        light: String,
        /// Theme for dark appearance.
        dark: String,
    },
}

impl Default for ThemeSelection {
    fn default() -> Self {
        Self::Dynamic {
            mode: ThemeMode::System,
            light: DEFAULT_LIGHT_THEME.to_owned(),
            dark: DEFAULT_DARK_THEME.to_owned(),
        }
    }
}

impl ThemeSelection {
    /// The theme name to show when the system is in `system` appearance.
    pub fn name_for(&self, system: Appearance) -> &str {
        match self {
            Self::Static(name) => name,
            Self::Dynamic { mode, light, dark } => match mode.appearance(system) {
                Appearance::Light => light,
                Appearance::Dark => dark,
            },
        }
    }

    /// The appearance this selection wants under `system`: the mode's for a dynamic selection,
    /// the system's for a static one (whose theme may be either; used to pick a fallback).
    pub fn appearance_for(&self, system: Appearance) -> Appearance {
        match self {
            Self::Static(_) => system,
            Self::Dynamic { mode, .. } => mode.appearance(system),
        }
    }

    fn from_content(content: ThemeSelectionContent) -> Self {
        match content {
            ThemeSelectionContent::Static(name) => Self::Static(name),
            ThemeSelectionContent::Dynamic { mode, light, dark } => Self::Dynamic {
                mode: mode.unwrap_or_default(),
                light: light.unwrap_or_else(|| DEFAULT_LIGHT_THEME.to_owned()),
                dark: dark.unwrap_or_else(|| DEFAULT_DARK_THEME.to_owned()),
            },
        }
    }
}

/// Theme settings, read with `ThemeSettings::get_global(cx)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThemeSettings {
    /// Which theme to show.
    pub selection: ThemeSelection,
}

impl Settings for ThemeSettings {
    const KEY: Option<&'static str> = Some("theme");
    type Content = ThemeSelectionContent;

    fn from_content(content: ThemeSelectionContent) -> Self {
        Self {
            selection: ThemeSelection::from_content(content),
        }
    }
}

oxikube_settings::register_settings!(ThemeSettings);

#[cfg(test)]
mod tests;
