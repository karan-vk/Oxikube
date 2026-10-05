//! `ThemeTokens` (from `oxikube_theme`) -> [`Tokens`]: the colours views read.

use super::tokens::{Appearance, Colors, Tokens};
use oxikube_theme::ThemeTokens;

impl From<oxikube_theme::Appearance> for Appearance {
    fn from(value: oxikube_theme::Appearance) -> Self {
        match value {
            oxikube_theme::Appearance::Light => Appearance::Light,
            oxikube_theme::Appearance::Dark => Appearance::Dark,
        }
    }
}

impl From<&ThemeTokens> for Tokens {
    /// Maps a theme onto the colours views draw with; sizes (spacing, radii, fonts) stay the
    /// defaults, because Zed theme files carry none.
    ///
    /// The content area (tables, lists, editors) uses the editor background and the panels use
    /// the surface colour, as in Zed; the accent is the theme's accent text colour and the
    /// status colours are its error / warning / success / info foregrounds.
    fn from(theme: &ThemeTokens) -> Self {
        let appearance = Appearance::from(theme.appearance);
        let c = &theme.colors;
        Self {
            colors: Colors {
                background: theme.editor.background,
                surface: c.surface,
                elevated_surface: c.elevated_surface,
                text: c.text,
                text_muted: c.text_muted,
                text_disabled: c.text_disabled,
                border: c.border,
                border_variant: c.border_variant,
                border_focused: c.border_focused,
                element: c.element,
                element_hover: c.element_hover,
                element_active: c.element_active,
                element_selected: c.element_selected,
                accent: c.text_accent,
                on_accent: c.on_accent,
                success: theme.status.success.foreground,
                warning: theme.status.warning.foreground,
                error: theme.status.error.foreground,
                info: theme.status.info.foreground,
                selection: c.selection,
            },
            ..Tokens::default_for(appearance)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_theme::{Appearance as ThemeAppearance, ThemeTokens};

    #[test]
    fn maps_colours_and_keeps_default_sizes() {
        let theme = ThemeTokens::fallback(ThemeAppearance::Light);
        let tokens = Tokens::from(theme);
        assert_eq!(tokens.appearance, Appearance::Light);
        assert_eq!(tokens.colors.background, theme.editor.background);
        assert_eq!(tokens.colors.surface, theme.colors.surface);
        assert_eq!(tokens.colors.accent, theme.colors.text_accent);
        assert_eq!(tokens.colors.error, theme.status.error.foreground);
        assert_eq!(tokens.colors.selection, theme.colors.selection);
        assert_eq!(tokens.spacing, Tokens::light().spacing);
        assert_eq!(tokens.font, Tokens::light().font);
    }
}
