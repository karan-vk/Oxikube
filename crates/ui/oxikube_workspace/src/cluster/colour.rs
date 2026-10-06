//! The colour a cluster's badge is drawn in, legible in the active theme.

use gpui::Hsla;
use oxikube_domain::{ClusterColour, ClusterPreset};
use oxikube_ui::{Appearance, Tokens};

/// The badge colour for `colour` under `tokens`.
///
/// The three preset colours (production red, staging amber, development green) are drawn with the
/// theme's own status colours (`error`, `warning`, `success`), which every theme keeps legible on
/// its surfaces. Any other colour the user picked is used as chosen, with its lightness pulled
/// into the legible band of the theme (not too dark on a dark surface, not too light on a light
/// one) so a `#000011` stripe does not vanish.
pub fn badge_colour(colour: ClusterColour, tokens: &Tokens) -> Hsla {
    match ClusterPreset::detect(Some(colour)) {
        ClusterPreset::Prod => tokens.colors.error,
        ClusterPreset::Staging => tokens.colors.warning,
        ClusterPreset::Dev => tokens.colors.success,
        ClusterPreset::None => legible(colour, tokens.appearance),
    }
}

/// Lightness bounds of a custom colour: `(min, max)` per appearance.
const fn band(appearance: Appearance) -> (f32, f32) {
    match appearance {
        Appearance::Dark => (0.55, 0.9),
        Appearance::Light => (0.2, 0.5),
    }
}

fn legible(colour: ClusterColour, appearance: Appearance) -> Hsla {
    let rgb = gpui::rgb(u32::from_be_bytes([0, colour.r, colour.g, colour.b]));
    let mut hsla: Hsla = rgb.into();
    let (min, max) = band(appearance);
    hsla.l = hsla.l.clamp(min, max);
    hsla
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_use_the_theme_status_colours() {
        for appearance in [Appearance::Dark, Appearance::Light] {
            let tokens = Tokens::default_for(appearance);
            assert_eq!(
                badge_colour(ClusterPreset::PROD_COLOUR, &tokens),
                tokens.colors.error
            );
            assert_eq!(
                badge_colour(ClusterPreset::STAGING_COLOUR, &tokens),
                tokens.colors.warning
            );
            assert_eq!(
                badge_colour(ClusterPreset::DEV_COLOUR, &tokens),
                tokens.colors.success
            );
        }
    }

    #[test]
    fn custom_colours_stay_in_the_legible_band() {
        let dark = Tokens::dark();
        let light = Tokens::light();
        let nearly_black = ClusterColour::rgb(0, 0, 0x11);
        let nearly_white = ClusterColour::rgb(0xff, 0xff, 0xee);
        assert!(badge_colour(nearly_black, &dark).l >= 0.55);
        assert!(badge_colour(nearly_white, &light).l <= 0.5);
        // A colour already in the band is left alone.
        let mid = ClusterColour::rgb(0x33, 0x99, 0xcc);
        let drawn = badge_colour(mid, &dark);
        let original: Hsla = gpui::rgb(0x3399cc).into();
        assert!((drawn.l - original.l.clamp(0.55, 0.9)).abs() < 1e-6);
        assert!((drawn.h - original.h).abs() < 1e-6, "the hue is the user's");
    }
}
