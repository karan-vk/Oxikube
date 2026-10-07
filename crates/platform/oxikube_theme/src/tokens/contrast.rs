//! WCAG 2.x contrast: the ratio between two colours and the smallest nudge that lifts one to a
//! target ratio on a set of backgrounds.

use gpui::{Hsla, Rgba};

/// WCAG relative luminance of `color` (alpha ignored).
pub(crate) fn luminance(color: Hsla) -> f32 {
    let Rgba { r, g, b, .. } = Rgba::from(color);
    let linear = |c: f32| {
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// The WCAG contrast ratio `(L1 + .05) / (L2 + .05)` of `foreground` over `background`
/// (`foreground` is composited onto `background` first, so a translucent colour is measured as
/// it appears).
///
/// Ranges from 1 (identical) to 21 (black on white); body text needs 4.5.
pub fn contrast_ratio(foreground: Hsla, background: Hsla) -> f32 {
    let background = Rgba::from(background);
    let (l1, l2) = (
        luminance(background.blend(foreground.into()).into()),
        luminance(background.into()),
    );
    (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05)
}

/// `color` with its lightness moved away from the backgrounds (lighter on dark ones, darker on
/// light ones), hue and saturation kept, until it reaches `minimum`:1 on every one of
/// `backgrounds`. A colour that already does is returned unchanged; one that cannot (pure
/// white or black still short) comes back as far as it goes.
pub(crate) fn with_min_contrast(color: Hsla, backgrounds: &[Hsla], minimum: f32) -> Hsla {
    let worst = |c: Hsla| {
        backgrounds
            .iter()
            .map(|bg| contrast_ratio(c, *bg))
            .fold(f32::INFINITY, f32::min)
    };
    if backgrounds.is_empty() || worst(color) >= minimum {
        return color;
    }
    let mean = backgrounds.iter().map(|bg| luminance(*bg)).sum::<f32>() / backgrounds.len() as f32;
    let step = if mean < 0.18 { 0.01 } else { -0.01 };
    let mut adjusted = color;
    while worst(adjusted) < minimum {
        let lightness = (adjusted.l + step).clamp(0.0, 1.0);
        if lightness == adjusted.l {
            break;
        }
        adjusted.l = lightness;
    }
    adjusted
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{black, rgb, white};

    #[test]
    fn the_ratio_matches_known_values() {
        assert!((contrast_ratio(black(), white()) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(white(), white()) - 1.0).abs() < 0.001);
        // #767676 on white is the classic 4.54:1 AA boundary.
        assert!((contrast_ratio(rgb(0x767676).into(), white()) - 4.54).abs() < 0.01);
    }

    #[test]
    fn a_colour_that_passes_is_untouched() {
        let ink: Hsla = rgb(0x242529).into();
        assert_eq!(with_min_contrast(ink, &[white()], 4.5), ink);
    }

    #[test]
    fn a_pale_colour_darkens_on_a_light_background_keeping_its_hue() {
        let yellow: Hsla = rgb(0xd2b67c).into();
        let backgrounds = [rgb(0xfafafa).into(), rgb(0xcacaca).into()];
        let fixed = with_min_contrast(yellow, &backgrounds, 4.5);
        assert!(fixed.l < yellow.l);
        assert_eq!((fixed.h, fixed.s), (yellow.h, yellow.s));
        assert!(
            backgrounds
                .iter()
                .all(|bg| contrast_ratio(fixed, *bg) >= 4.5)
        );
    }

    #[test]
    fn a_dim_colour_lightens_on_a_dark_background() {
        let magenta: Hsla = rgb(0xc678dd).into();
        let backgrounds = [rgb(0x282c33).into(), rgb(0x3f4552).into()];
        let fixed = with_min_contrast(magenta, &backgrounds, 4.5);
        assert!(fixed.l > magenta.l);
        assert!(
            backgrounds
                .iter()
                .all(|bg| contrast_ratio(fixed, *bg) >= 4.5)
        );
    }
}
