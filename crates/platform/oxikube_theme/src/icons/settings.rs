//! The `icon_theme` setting: an icon theme name, or a mode with a light and a dark one (Zed's
//! shape, the same as `theme`).
//!
//! ```json
//! "icon_theme": "Lucide"
//! "icon_theme": { "mode": "system", "light": "Material Light", "dark": "Material" }
//! ```

use super::theme::DEFAULT_ICON_THEME;
use crate::settings::{ThemeSelection, ThemeSelectionContent};
use oxikube_settings::Settings;

/// Icon-theme settings, read with `IconThemeSettings::get_global(cx)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconThemeSettings {
    /// Which icon theme to show; a name that is not installed shows Lucide.
    pub selection: ThemeSelection,
}

impl Default for IconThemeSettings {
    fn default() -> Self {
        Self {
            selection: ThemeSelection::Static(DEFAULT_ICON_THEME.to_owned()),
        }
    }
}

impl Settings for IconThemeSettings {
    const KEY: Option<&'static str> = Some("icon_theme");
    type Content = ThemeSelectionContent;

    fn from_content(content: ThemeSelectionContent) -> Self {
        let or_default =
            |name: Option<String>| name.unwrap_or_else(|| DEFAULT_ICON_THEME.to_owned());
        let selection = match content {
            ThemeSelectionContent::Static(name) => ThemeSelection::Static(name),
            ThemeSelectionContent::Dynamic { mode, light, dark } => ThemeSelection::Dynamic {
                mode: mode.unwrap_or_default(),
                light: or_default(light),
                dark: or_default(dark),
            },
        };
        Self { selection }
    }
}

oxikube_settings::register_settings!(IconThemeSettings);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::{Appearance, ThemeMode};

    #[test]
    fn a_name_or_a_mode_with_lucide_for_what_is_unset() {
        let content: ThemeSelectionContent = serde_json::from_str(r#""Material""#).unwrap();
        assert_eq!(
            IconThemeSettings::from_content(content).selection,
            ThemeSelection::Static("Material".into())
        );
        let content: ThemeSelectionContent =
            serde_json::from_str(r#"{ "mode": "light", "dark": "Material" }"#).unwrap();
        let selection = IconThemeSettings::from_content(content).selection;
        assert_eq!(
            selection,
            ThemeSelection::Dynamic {
                mode: ThemeMode::Light,
                light: DEFAULT_ICON_THEME.into(),
                dark: "Material".into(),
            }
        );
        assert_eq!(selection.name_for(Appearance::Dark), DEFAULT_ICON_THEME);
    }
}
