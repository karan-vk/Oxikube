//! [`SecretText`]: a decoded Secret value that cannot leak through formatting.

use std::fmt;

use secrecy::{ExposeSecret as _, SecretString};

/// What `Debug` and `Display` print for a [`SecretText`].
pub const STARS: &str = "***";

/// A Secret value in memory: text that prints as `***` in `Debug` and `Display`, is zeroised
/// when dropped, and is read only through the explicit [`expose`](Self::expose).
///
/// It deliberately has no `Serialize`, `Deref` or `AsRef`: a decoded value reaches the screen
/// or the editor buffer on purpose, never through a derived `Debug`, a log field or a JSON dump.
#[derive(Clone)]
pub struct SecretText(SecretString);

impl SecretText {
    /// Wraps `value`.
    pub fn new(value: impl Into<String>) -> Self {
        Self(SecretString::from(value.into()))
    }

    /// The text. Calling this is the audit point for "does this leak?".
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }

    /// The length in bytes.
    pub fn len(&self) -> usize {
        self.expose().len()
    }

    /// Whether the text is empty.
    pub fn is_empty(&self) -> bool {
        self.expose().is_empty()
    }
}

impl PartialEq for SecretText {
    fn eq(&self, other: &Self) -> bool {
        self.expose() == other.expose()
    }
}

impl Eq for SecretText {}

impl fmt::Debug for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(STARS)
    }
}

impl fmt::Display for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(STARS)
    }
}

impl From<String> for SecretText {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_print_stars() {
        let secret = SecretText::new("hunter2");
        assert_eq!(format!("{secret:?}"), "***");
        assert_eq!(format!("{secret}"), "***");
        assert_eq!(secret.expose(), "hunter2");
        assert_eq!(secret.len(), 7);
    }

    #[test]
    fn a_derived_debug_around_it_leaks_nothing() {
        #[derive(Debug)]
        struct Holder {
            value: SecretText,
        }
        let held = Holder {
            value: SecretText::new("hunter2"),
        };
        assert_eq!(held.value.len(), 7);
        let printed = format!("{held:?} {held:#?}");
        assert!(!printed.contains("hunter2"), "{printed}");
    }
}
