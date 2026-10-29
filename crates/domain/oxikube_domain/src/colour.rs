//! [`ClusterColour`]: the accent colour a user gives a cluster.
//!
//! A cluster session carries an optional colour (Lens/Freelens-style hotbar dot, tab
//! stripe, status-bar badge; a red preset for production). It is plain sRGB with no
//! alpha, written and persisted as `#rrggbb` so settings files stay readable. Turning it
//! into a GPUI colour is the UI layer's job.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// An opaque sRGB colour, written `#rrggbb`.
///
/// Parsing accepts `#rrggbb`, the short form `#rgb` (each digit doubled), in either case, and
/// the [`ClusterPalette`] names (`red`, `orange`, `green`, `blue`, `purple`, `gray`); a name
/// resolves to the palette's hex. `Display` and serde always produce lowercase `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ClusterColour {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

/// Text that is not a `#rrggbb` or `#rgb` colour.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "invalid cluster colour {0:?}: expected #rrggbb, #rgb or a palette name \
     (red, orange, green, blue, purple, gray)"
)]
pub struct InvalidColour(pub String);

impl ClusterColour {
    /// A colour from its channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parses `#rrggbb`, `#rgb` or a [`ClusterPalette`] name (case-insensitive).
    ///
    /// # Errors
    ///
    /// [`InvalidColour`] for anything else (unknown name, wrong length, non-hex digits).
    pub fn parse(text: &str) -> Result<Self, InvalidColour> {
        let invalid = || InvalidColour(text.to_owned());
        let trimmed = text.trim();
        if let Some(palette) = ClusterPalette::from_name(trimmed) {
            return Ok(palette.colour());
        }
        let hex = trimmed.strip_prefix('#').ok_or_else(invalid)?;
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let digit = |i: usize| u8::from_str_radix(&hex[i..=i], 16).map_err(|_| invalid());
        let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| invalid());
        match hex.len() {
            3 => Ok(Self::rgb(digit(0)? * 17, digit(1)? * 17, digit(2)? * 17)),
            6 => Ok(Self::rgb(pair(0)?, pair(2)?, pair(4)?)),
            _ => Err(invalid()),
        }
    }
}

/// The six named colours a cluster can be given instead of a hex value.
///
/// Red, orange and green are the colours of the production, staging and development presets
/// ([`ClusterPreset`](crate::ClusterPreset)); the UI draws them with the theme's own status
/// tokens, so a theme keeps them legible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClusterPalette {
    /// Production red.
    Red,
    /// Staging amber.
    Orange,
    /// Development green.
    Green,
    /// Blue.
    Blue,
    /// Purple.
    Purple,
    /// Neutral gray.
    Gray,
}

impl ClusterPalette {
    /// Every palette colour, in the order the documentation lists them.
    pub const ALL: [ClusterPalette; 6] = [
        Self::Red,
        Self::Orange,
        Self::Green,
        Self::Blue,
        Self::Purple,
        Self::Gray,
    ];

    /// The name written in settings.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Purple => "purple",
            Self::Gray => "gray",
        }
    }

    /// The palette colour called `name` (case-insensitive; `grey` is accepted for `gray`).
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.trim();
        Self::ALL
            .into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(name))
            .or_else(|| name.eq_ignore_ascii_case("grey").then_some(Self::Gray))
    }

    /// The sRGB value the name stands for.
    pub const fn colour(self) -> ClusterColour {
        match self {
            Self::Red => ClusterColour::rgb(0xe5, 0x48, 0x4d),
            Self::Orange => ClusterColour::rgb(0xf5, 0xa5, 0x24),
            Self::Green => ClusterColour::rgb(0x30, 0xa4, 0x6c),
            Self::Blue => ClusterColour::rgb(0x3e, 0x63, 0xdd),
            Self::Purple => ClusterColour::rgb(0x8e, 0x4e, 0xc6),
            Self::Gray => ClusterColour::rgb(0x8b, 0x8d, 0x98),
        }
    }

    /// The palette colour `colour` is exactly, if any.
    pub fn of(colour: ClusterColour) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.colour() == colour)
    }
}

impl fmt::Display for ClusterColour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

impl FromStr for ClusterColour {
    type Err = InvalidColour;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for ClusterColour {
    type Error = InvalidColour;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ClusterColour> for String {
    fn from(value: ClusterColour) -> Self {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parses_long_and_short_forms() {
        assert_eq!(
            ClusterColour::parse("#E5484D"),
            Ok(ClusterColour::rgb(0xe5, 0x48, 0x4d))
        );
        assert_eq!(
            ClusterColour::parse(" #f00 "),
            Ok(ClusterColour::rgb(255, 0, 0))
        );
        assert_eq!(ClusterColour::rgb(1, 2, 255).to_string(), "#0102ff");
    }

    #[test]
    fn palette_names_resolve_to_their_hex_in_any_case() {
        for palette in ClusterPalette::ALL {
            assert_eq!(ClusterColour::parse(palette.name()), Ok(palette.colour()));
            assert_eq!(ClusterPalette::of(palette.colour()), Some(palette));
        }
        assert_eq!(
            ClusterColour::parse(" Purple "),
            Ok(ClusterPalette::Purple.colour())
        );
        assert_eq!(
            ClusterPalette::from_name("grey"),
            Some(ClusterPalette::Gray)
        );
        assert!(ClusterColour::parse("crimson").is_err());
    }

    #[test]
    fn rejects_malformed_text() {
        for bad in [
            "", "#", "f00", "#ff00", "#gg0000", "#ff00000", "#+1f", "#ü0",
        ] {
            assert!(ClusterColour::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn serde_uses_the_hex_string() {
        let colour = ClusterColour::rgb(0x12, 0xab, 0xef);
        let json = serde_json::to_string(&colour).unwrap();
        assert_eq!(json, r##""#12abef""##);
        assert_eq!(
            serde_json::from_str::<ClusterColour>(&json).unwrap(),
            colour
        );
        assert!(serde_json::from_str::<ClusterColour>(r#""crimson""#).is_err());
    }

    proptest! {
        #[test]
        fn display_round_trips(r: u8, g: u8, b: u8) {
            let colour = ClusterColour::rgb(r, g, b);
            prop_assert_eq!(colour.to_string().parse::<ClusterColour>(), Ok(colour));
        }

        #[test]
        fn arbitrary_text_never_panics(s in any::<String>()) {
            let _ = ClusterColour::parse(&s);
        }
    }
}
