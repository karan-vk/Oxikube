//! The chart colours, derived from the active [`Tokens`].
//!
//! Status colours (good, warn, critical, info) and the grid, axis and text colours are the theme's
//! own. The categorical colours for per-container and per-pod series are a hue wheel whose
//! lightness and saturation follow the appearance, chosen so every colour keeps at least 3:1
//! contrast (WCAG 1.4.11, graphical objects) against the chart backgrounds in the built-in dark and
//! light tokens and the bundled One Dark and One Light themes; `palette::tests` checks it.

use super::data::{SeriesColor, Tone};
use crate::tokens::{Appearance, Tokens};
use gpui::{Hsla, hsla};

/// How many categorical colours before they repeat.
pub const CATEGORICAL: usize = 8;

/// Hues, in degrees, of the categorical colours: blue, orange, green, purple, yellow, teal, pink,
/// lime. Neighbouring entries are far apart on the wheel so adjacent series are told apart.
const HUES: [f32; CATEGORICAL] = [212.0, 28.0, 140.0, 280.0, 48.0, 178.0, 336.0, 84.0];

/// Everything a chart paints with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartPalette {
    /// Categorical series colours, in series order.
    pub series: [Hsla; CATEGORICAL],
    /// Gridlines.
    pub grid: Hsla,
    /// The baseline under the plot.
    pub baseline: Hsla,
    /// Tick labels and the legend text.
    pub axis_text: Hsla,
    /// The chart surface (marker outlines, tooltips draw on it).
    pub background: Hsla,
    /// [`Tone::Good`].
    pub good: Hsla,
    /// [`Tone::Warn`].
    pub warn: Hsla,
    /// [`Tone::Critical`].
    pub critical: Hsla,
    /// [`Tone::Info`].
    pub info: Hsla,
    /// [`Tone::Muted`].
    pub muted: Hsla,
    /// Opacity of an area's fill under its line.
    pub area_alpha: f32,
    /// Opacity of a status band.
    pub band_alpha: f32,
}

impl ChartPalette {
    /// The palette for `tokens`.
    pub fn from_tokens(tokens: &Tokens) -> Self {
        let c = &tokens.colors;
        let (saturation, lightness) = match tokens.appearance {
            Appearance::Dark => (0.62, 0.66),
            Appearance::Light => (0.70, 0.31),
        };
        Self {
            series: HUES.map(|hue| hsla(hue / 360.0, saturation, lightness, 1.0)),
            grid: c.border_variant,
            baseline: c.border,
            axis_text: c.text_muted,
            background: c.background,
            good: c.success,
            warn: c.warning,
            critical: c.error,
            info: c.info,
            muted: c.text_muted,
            area_alpha: 0.18,
            band_alpha: 0.12,
        }
    }

    /// The colour of a [`Tone`].
    pub fn tone(&self, tone: Tone) -> Hsla {
        match tone {
            Tone::Good => self.good,
            Tone::Warn => self.warn,
            Tone::Critical => self.critical,
            Tone::Info => self.info,
            Tone::Muted => self.muted,
        }
    }

    /// The colour of the series at position `index`.
    pub fn categorical(&self, index: usize) -> Hsla {
        self.series[index % CATEGORICAL]
    }

    /// Resolves a series' colour source; `index` is its position in the chart.
    pub fn resolve(&self, color: SeriesColor, index: usize) -> Hsla {
        match color {
            SeriesColor::Auto => self.categorical(index),
            SeriesColor::Tone(tone) => self.tone(tone),
            SeriesColor::Fixed(color) => color,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_theme::tokens::contrast_ratio;
    use oxikube_theme::{Appearance as ThemeAppearance, ThemeRegistry, ThemeTokens};

    fn token_sets() -> Vec<(String, Tokens)> {
        let mut sets = vec![
            ("Tokens::dark".to_owned(), Tokens::dark()),
            ("Tokens::light".to_owned(), Tokens::light()),
            (
                "fallback dark".to_owned(),
                Tokens::from(ThemeTokens::fallback(ThemeAppearance::Dark)),
            ),
            (
                "fallback light".to_owned(),
                Tokens::from(ThemeTokens::fallback(ThemeAppearance::Light)),
            ),
        ];
        let registry = ThemeRegistry::with_bundled();
        for name in ["One Dark", "One Light"] {
            if let Some(theme) = registry.get(name) {
                sets.push((name.to_owned(), Tokens::from(&*theme)));
            }
        }
        sets
    }

    #[test]
    fn every_series_colour_has_3_to_1_contrast_on_the_chart_surfaces() {
        let mut failures = Vec::new();
        for (set, tokens) in token_sets() {
            let palette = ChartPalette::from_tokens(&tokens);
            let surfaces = [
                ("background", tokens.colors.background),
                ("surface", tokens.colors.surface),
                ("elevated_surface", tokens.colors.elevated_surface),
            ];
            for (i, colour) in palette.series.iter().enumerate() {
                for (name, surface) in surfaces {
                    let ratio = contrast_ratio(*colour, surface);
                    if ratio < 3.0 {
                        failures.push(format!("{set}: series {i} on {name} = {ratio:.2}"));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "below 3:1:\n{}", failures.join("\n"));
    }

    #[test]
    fn status_colours_have_3_to_1_contrast_too() {
        for (set, tokens) in token_sets() {
            let palette = ChartPalette::from_tokens(&tokens);
            for tone in [
                Tone::Good,
                Tone::Warn,
                Tone::Critical,
                Tone::Info,
                Tone::Muted,
            ] {
                let ratio = contrast_ratio(palette.tone(tone), tokens.colors.background);
                assert!(ratio >= 3.0, "{set}: {tone:?} = {ratio:.2}");
            }
        }
    }

    #[test]
    fn categorical_colours_are_distinct() {
        for (set, tokens) in token_sets() {
            let palette = ChartPalette::from_tokens(&tokens);
            for (i, a) in palette.series.iter().enumerate() {
                for b in &palette.series[i + 1..] {
                    assert!((a.h - b.h).abs() > 0.03, "{set}: two colours share a hue");
                }
            }
        }
    }

    #[test]
    fn colours_resolve_by_position_and_wrap() {
        let palette = ChartPalette::from_tokens(&Tokens::dark());
        assert_eq!(palette.resolve(SeriesColor::Auto, 0), palette.series[0]);
        assert_eq!(
            palette.resolve(SeriesColor::Auto, CATEGORICAL),
            palette.series[0]
        );
        assert_eq!(
            palette.resolve(SeriesColor::Tone(Tone::Critical), 3),
            palette.critical
        );
        let red = hsla(0.0, 1.0, 0.5, 1.0);
        assert_eq!(palette.resolve(SeriesColor::Fixed(red), 3), red);
    }

    #[test]
    fn dark_and_light_differ() {
        let dark = ChartPalette::from_tokens(&Tokens::dark());
        let light = ChartPalette::from_tokens(&Tokens::light());
        assert_ne!(dark.series[0], light.series[0]);
        assert_ne!(dark.grid, light.grid);
    }
}
