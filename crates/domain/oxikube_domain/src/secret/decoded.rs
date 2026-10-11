//! [`DecodedSecret`]: a Secret's `data` read as text, and written back as base64.

use base64::Engine as _;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use serde_json::{Map, Value};

use super::kind::SecretType;
use super::text::SecretText;
use crate::resource::Resource;

/// Standard base64, padded on write and lenient about padding on read (the API server writes
/// padded, hand-made objects often do not).
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// What starts the placeholder the editor shows for a value that is not text: `(binary, 12 bytes)`.
pub const BINARY_PREFIX: &str = "(binary, ";

/// What stands for a value that is masked (the editor's and the detail's placeholder). It is never
/// encoded as a value: [`from_decoded`](super::from_decoded) refuses a Secret that still has it.
pub const MASKED_VALUE: &str = "(hidden)";

/// Why an object is not read or written as a Secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// The object is not a core `Secret`.
    #[error("expected a Secret, got {0}")]
    NotASecret(String),
    /// A value under `data` is not a string.
    #[error("the value of `{0}` is not text")]
    NotText(String),
    /// A value under `data` is still the masked placeholder: it was never revealed, so there is
    /// nothing to encode.
    #[error("the value of `{0}` is still masked: reveal it or replace it")]
    Masked(String),
}

/// One value of a Secret, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedValue {
    /// Valid UTF-8 text (control characters other than tab and line breaks count as binary).
    Text(SecretText),
    /// Anything else, or base64 that does not decode: shown as its size, carried through as the
    /// original base64 so applying never changes it.
    Binary {
        /// The decoded size in bytes (the length of the text when it was not valid base64).
        len: usize,
        /// The original `data` string.
        encoded: SecretText,
    },
}

impl DecodedValue {
    /// Decodes one `data` string.
    pub fn from_base64(encoded: &str) -> Self {
        let compact: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
        let Ok(bytes) = BASE64.decode(compact.as_bytes()) else {
            return DecodedValue::Binary {
                len: encoded.len(),
                encoded: SecretText::new(encoded),
            };
        };
        match String::from_utf8(bytes) {
            Ok(text) if is_displayable(&text) => DecodedValue::Text(SecretText::new(text)),
            Ok(text) => DecodedValue::Binary {
                len: text.len(),
                encoded: SecretText::new(encoded),
            },
            Err(error) => DecodedValue::Binary {
                len: error.as_bytes().len(),
                encoded: SecretText::new(encoded),
            },
        }
    }

    /// The text, `None` for a binary value.
    pub fn text(&self) -> Option<&str> {
        match self {
            DecodedValue::Text(text) => Some(text.expose()),
            DecodedValue::Binary { .. } => None,
        }
    }

    /// The placeholder for a binary value (`(binary, 12 bytes)`), `None` for text.
    pub fn placeholder(&self) -> Option<String> {
        match self {
            DecodedValue::Text(_) => None,
            DecodedValue::Binary { len, .. } => Some(placeholder(*len)),
        }
    }

    /// What the editor and the detail show for this value: its text, or the binary placeholder.
    pub fn shown(&self) -> String {
        match self {
            DecodedValue::Text(text) => text.expose().to_owned(),
            DecodedValue::Binary { len, .. } => placeholder(*len),
        }
    }

    /// The `data` string this value is written as.
    pub fn to_base64(&self) -> String {
        match self {
            DecodedValue::Text(text) => BASE64.encode(text.expose().as_bytes()),
            DecodedValue::Binary { encoded, .. } => encoded.expose().to_owned(),
        }
    }

    /// The size of the decoded value in bytes.
    pub fn len(&self) -> usize {
        match self {
            DecodedValue::Text(text) => text.len(),
            DecodedValue::Binary { len, .. } => *len,
        }
    }

    /// Whether the decoded value is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// `(binary, 12 bytes)`.
pub fn placeholder(len: usize) -> String {
    let noun = if len == 1 { "byte" } else { "bytes" };
    format!("{BINARY_PREFIX}{len} {noun})")
}

/// Whether `text` is a placeholder [`placeholder`] writes.
pub fn is_placeholder(text: &str) -> bool {
    text.strip_prefix(BINARY_PREFIX)
        .and_then(|rest| rest.strip_suffix(')'))
        .and_then(|rest| rest.strip_suffix(" bytes").or(rest.strip_suffix(" byte")))
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether `text` is safe to show and edit: no control characters but tab and line breaks. A
/// value that is not is read as binary and shown as a size, so it cannot be typed as text.
pub fn is_displayable(text: &str) -> bool {
    text.chars()
        .all(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
}

/// One key of a decoded Secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedEntry {
    /// The key (`password`, `tls.crt`).
    pub key: String,
    /// Its value.
    pub value: DecodedValue,
}

/// A Secret with its values decoded: what "reveal" and "edit decoded" work on. Lives in memory
/// only; `Debug` prints the keys and `***`, never a value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecodedSecret {
    /// The Secret's type.
    pub secret_type: SecretType,
    /// The keys, in the order the object lists them (`data` first, then `stringData` keys that
    /// `data` does not have).
    pub entries: Vec<DecodedEntry>,
}

impl DecodedSecret {
    /// Decodes `resource`.
    ///
    /// # Errors
    ///
    /// [`SecretError::NotASecret`] for any other kind.
    pub fn from_resource(resource: &Resource) -> Result<Self, SecretError> {
        Self::from_value(&resource.to_value())
    }

    /// Decodes the JSON of a Secret: `data` is base64, `stringData` is plain text (the API never
    /// serves it, but a manifest may carry it, and it wins over `data` for the same key).
    ///
    /// # Errors
    ///
    /// [`SecretError::NotASecret`] for any other object.
    pub fn from_value(object: &Value) -> Result<Self, SecretError> {
        if !is_secret(object) {
            return Err(SecretError::NotASecret(describe(object)));
        }
        let secret_type = SecretType::parse(object.get("type").and_then(Value::as_str));
        let mut entries: Vec<DecodedEntry> = Vec::new();
        if let Some(data) = object.get("data").and_then(Value::as_object) {
            for (key, value) in data {
                if let Some(encoded) = value.as_str() {
                    entries.push(DecodedEntry {
                        key: key.clone(),
                        value: DecodedValue::from_base64(encoded),
                    });
                }
            }
        }
        if let Some(plain) = object.get("stringData").and_then(Value::as_object) {
            for (key, value) in plain {
                let Some(text) = value.as_str() else { continue };
                let value = DecodedValue::Text(SecretText::new(text));
                match entries.iter_mut().find(|entry| entry.key == *key) {
                    Some(entry) => entry.value = value,
                    None => entries.push(DecodedEntry {
                        key: key.clone(),
                        value,
                    }),
                }
            }
        }
        Ok(Self {
            secret_type,
            entries,
        })
    }

    /// The value under `key`.
    pub fn get(&self, key: &str) -> Option<&DecodedValue> {
        self.entries
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| &entry.value)
    }

    /// The text under `key`, `None` when absent or binary.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(DecodedValue::text)
    }

    /// The `data` object these entries are written as: every value base64.
    pub fn encode(&self) -> Value {
        Value::Object(
            self.entries
                .iter()
                .map(|entry| (entry.key.clone(), Value::String(entry.value.to_base64())))
                .collect::<Map<String, Value>>(),
        )
    }
}

/// Whether `object` is a core `Secret` (`apiVersion: v1`, `kind: Secret`).
pub fn is_secret(object: &Value) -> bool {
    object.get("apiVersion").and_then(Value::as_str) == Some("v1")
        && object.get("kind").and_then(Value::as_str) == Some("Secret")
}

fn describe(object: &Value) -> String {
    let kind = object.get("kind").and_then(Value::as_str).unwrap_or("?");
    match object.get("apiVersion").and_then(Value::as_str) {
        Some(version) => format!("{version}/{kind}"),
        None => kind.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn secret(data: Value) -> Value {
        json!({"apiVersion": "v1", "kind": "Secret", "metadata": {"name": "s"}, "data": data})
    }

    #[test]
    fn text_values_decode_and_encode_back() {
        let object = secret(json!({"password": "aHVudGVyMg==", "multi": "YQpiCg=="}));
        let decoded = DecodedSecret::from_value(&object).unwrap();
        assert_eq!(decoded.text("password"), Some("hunter2"));
        assert_eq!(decoded.text("multi"), Some("a\nb\n"));
        assert_eq!(decoded.encode(), object["data"]);
    }

    #[test]
    fn binary_values_are_carried_unchanged() {
        // 0xff 0xfe is not UTF-8; "AAEC" is 00 01 02, valid UTF-8 but control characters.
        let object = secret(json!({"blob": "//4=", "ctl": "AAEC", "bad": "!!not base64!!"}));
        let decoded = DecodedSecret::from_value(&object).unwrap();
        for key in ["blob", "ctl", "bad"] {
            assert!(decoded.get(key).unwrap().text().is_none(), "{key}");
        }
        assert_eq!(
            decoded.get("blob").unwrap().placeholder().unwrap(),
            "(binary, 2 bytes)"
        );
        assert_eq!(decoded.get("ctl").unwrap().len(), 3);
        assert_eq!(decoded.encode(), object["data"]);
    }

    #[test]
    fn padding_and_line_breaks_in_data_are_tolerated() {
        let object = secret(json!({"a": "aHVudGVyMg", "b": "aHVu\ndGVy\nMg=="}));
        let decoded = DecodedSecret::from_value(&object).unwrap();
        assert_eq!(decoded.text("a"), Some("hunter2"));
        assert_eq!(decoded.text("b"), Some("hunter2"));
    }

    #[test]
    fn string_data_wins_and_adds_keys() {
        let mut object = secret(json!({"a": "b2xk"}));
        object["stringData"] = json!({"a": "new", "b": "added"});
        let decoded = DecodedSecret::from_value(&object).unwrap();
        assert_eq!(decoded.text("a"), Some("new"));
        assert_eq!(decoded.text("b"), Some("added"));
        assert_eq!(decoded.entries.len(), 2);
    }

    #[test]
    fn the_type_is_read() {
        let mut object = secret(json!({}));
        object["type"] = json!("kubernetes.io/tls");
        assert_eq!(
            DecodedSecret::from_value(&object).unwrap().secret_type,
            SecretType::Tls
        );
    }

    #[test]
    fn another_kind_is_refused() {
        let config = json!({"apiVersion": "v1", "kind": "ConfigMap", "data": {}});
        assert_eq!(
            DecodedSecret::from_value(&config),
            Err(SecretError::NotASecret("v1/ConfigMap".into()))
        );
    }

    #[test]
    fn debug_never_prints_a_value() {
        let object = secret(json!({"password": "aHVudGVyMg==", "blob": "//4="}));
        let decoded = DecodedSecret::from_value(&object).unwrap();
        let printed = format!("{decoded:?} {decoded:#?}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(!printed.contains("aHVudGVyMg"), "{printed}");
        assert!(!printed.contains("//4="), "{printed}");
        assert!(printed.contains("password") && printed.contains("***"));
    }

    #[test]
    fn placeholders_are_recognised() {
        assert!(is_placeholder("(binary, 12 bytes)"));
        assert!(is_placeholder("(binary, 1 byte)"));
        assert!(!is_placeholder("(binary, x bytes)"));
        assert!(!is_placeholder("(binary,  bytes)"));
        assert!(!is_placeholder("(hidden)"));
        assert!(!is_placeholder("binary, 12 bytes"));
    }
}
