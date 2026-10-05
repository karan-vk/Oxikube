//! Hex colour parsing for theme files: `#rgb`, `#rgba`, `#rrggbb` and `#rrggbbaa`.

use gpui::{Hsla, Rgba};

/// Parses a theme colour string into [`Hsla`].
///
/// Accepts the four hex forms GPUI knows (case-insensitive, surrounding whitespace ignored).
/// Six-digit colours are opaque. The error text names the offending value.
pub fn parse_color(text: &str) -> Result<Hsla, String> {
    Rgba::try_from(text)
        .map(Hsla::from)
        .map_err(|err| err.to_string())
}

/// `#rrggbbaa` for `color` (what Zed theme files and gpui-component's `ThemeConfig` read).
///
/// Channels are rounded, not truncated: an `Hsla` made from `#0d1016` holds the channels as
/// slightly-off `f32`s, and truncating would print `#0d1015`.
pub fn to_hex(color: Hsla) -> String {
    let Rgba { r, g, b, a } = Rgba::from(color);
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        byte(r),
        byte(g),
        byte(b),
        byte(a)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::rgba;

    #[test]
    fn parses_six_and_eight_digit_hex() {
        assert_eq!(parse_color("#74ade8"), Ok(Hsla::from(rgba(0x74ade8ff))));
        assert_eq!(parse_color("#74ade83d"), Ok(Hsla::from(rgba(0x74ade83d))));
        assert_eq!(parse_color(" #ABCDEF99 "), Ok(Hsla::from(rgba(0xabcdef99))));
    }

    #[test]
    fn parses_short_forms() {
        assert_eq!(parse_color("#f80"), Ok(Hsla::from(rgba(0xff8800ff))));
        assert_eq!(parse_color("#f808"), Ok(Hsla::from(rgba(0xff880088))));
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            "",
            "red",
            "74ade8",
            "#74ade",
            "#74ade8f",
            "#xyzxyz",
            "#74ade8ff00",
            "#é",
        ] {
            assert!(parse_color(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn to_hex_round_trips_every_channel_value() {
        for value in 0..=255u32 {
            for shift in [24, 16, 8, 0] {
                let text = format!("#{:08x}", (value << shift) | 0x0001_0203);
                assert_eq!(to_hex(parse_color(&text).unwrap()), text);
            }
        }
    }
}
