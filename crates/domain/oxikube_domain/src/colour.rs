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
/// Parsing accepts `#rrggbb` and the short form `#rgb` (each digit doubled), in either
/// case; `Display` and serde always produce lowercase `#rrggbb`.
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
#[error("invalid cluster colour {0:?}: expected #rrggbb or #rgb")]
pub struct InvalidColour(pub String);

impl ClusterColour {
    /// A colour from its channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parses `#rrggbb` or `#rgb`.
    ///
    /// # Errors
    ///
    /// [`InvalidColour`] for anything else (missing `#`, wrong length, non-hex digits).
    pub fn parse(text: &str) -> Result<Self, InvalidColour> {
        let invalid = || InvalidColour(text.to_owned());
        let hex = text.trim().strip_prefix('#').ok_or_else(invalid)?;
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
        assert!(serde_json::from_str::<ClusterColour>(r#""red""#).is_err());
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
