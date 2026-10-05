//! Syntax highlight styles and per-collaborator player colours.

use gpui::Hsla;
use std::collections::BTreeMap;

/// Italic or upright, from a theme's `font_style`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontStyle {
    /// Upright.
    Normal,
    /// Italic.
    Italic,
    /// Oblique.
    Oblique,
}

/// How one syntax capture (`keyword`, `string.escape`, ...) is drawn. Unset parts inherit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SyntaxStyle {
    /// Text colour.
    pub color: Option<Hsla>,
    /// Italic / oblique.
    pub font_style: Option<FontStyle>,
    /// Weight on the CSS 100..=900 scale.
    pub font_weight: Option<f32>,
}

/// Syntax highlight styles by dotted capture name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SyntaxTheme {
    /// Styles by capture name, in name order.
    pub styles: BTreeMap<String, SyntaxStyle>,
}

impl SyntaxTheme {
    /// The style for `name`, falling back to its dotted ancestors (`string.escape` -> `string`).
    pub fn style(&self, name: &str) -> Option<&SyntaxStyle> {
        let mut name = name;
        loop {
            if let Some(style) = self.styles.get(name) {
                return Some(style);
            }
            name = name.rsplit_once('.')?.0;
        }
    }
}

/// Colours of one collaborator/cursor. Index 0 is the local user.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerColor {
    /// Cursor colour.
    pub cursor: Hsla,
    /// Background of the player's avatar or highlight.
    pub background: Hsla,
    /// Selection colour.
    pub selection: Hsla,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_falls_back_to_dotted_ancestors() {
        let mut theme = SyntaxTheme::default();
        let color = gpui::hsla(0.5, 0.5, 0.5, 1.0);
        theme.styles.insert(
            "string".into(),
            SyntaxStyle {
                color: Some(color),
                ..Default::default()
            },
        );
        assert_eq!(
            theme.style("string.escape").and_then(|s| s.color),
            Some(color)
        );
        assert_eq!(theme.style("string").and_then(|s| s.color), Some(color));
        assert!(theme.style("keyword").is_none());
        assert!(theme.style("").is_none());
    }
}
