//! [`SecretValue`]: a decoded Secret value that cannot leak through `Debug`, logs or serde.

use std::fmt;

/// What a masked value reads as: a fixed length, so the mask does not give the value's away.
pub const MASK: &str = "••••••••";

/// A decoded Secret value (non-negotiable 5: decoded values live in memory only).
///
/// It has no `Display`, no `Serialize` and a `Debug` that prints no content, so it cannot reach a
/// log line, an audit record or a snapshot by accident. The only way to the text is
/// [`reveal`](Self::reveal), which a view calls when the user asks for it.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(Box<str>);

impl SecretValue {
    /// A value holding `text`.
    pub fn new(text: impl Into<Box<str>>) -> Self {
        Self(text.into())
    }

    /// A value decoded from the base64 text of a Secret's `data` entry. Bytes that are not UTF-8
    /// are shown lossily; text that is not base64 is kept as it is (a hand-written fixture).
    pub fn from_base64(encoded: &str) -> Self {
        use base64::Engine as _;
        let cleaned: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
        match base64::engine::general_purpose::STANDARD.decode(cleaned.as_bytes()) {
            Ok(bytes) => Self(String::from_utf8_lossy(&bytes).into_owned().into()),
            Err(_) => Self(encoded.into()),
        }
    }

    /// The text. Call it only to show the value the user asked to see.
    pub fn reveal(&self) -> &str {
        &self.0
    }

    /// The length in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the value is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_prints_no_content() {
        let value = SecretValue::from_base64("aHVudGVyMg==");
        assert_eq!(value.reveal(), "hunter2");
        assert!(!format!("{value:?}").contains("hunter2"));
        assert!(!format!("{:?}", Some(value)).contains("hunter2"));
    }

    #[test]
    fn undecodable_text_is_kept_and_binary_is_lossy() {
        assert_eq!(
            SecretValue::from_base64("not base64!").reveal(),
            "not base64!"
        );
        assert_eq!(SecretValue::from_base64("/w==").reveal(), "\u{fffd}");
    }
}
