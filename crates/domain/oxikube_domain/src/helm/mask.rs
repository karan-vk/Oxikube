//! Masking of release values for display (E16-S05), following the secret rules of E12-S11.
//!
//! A values tree shows a credential-looking key's value as [`MASK`] until the user asks for it
//! (`helm::RevealValues`). The shared scrubber ([`crate::redact`]) also runs over every other
//! string, so a bearer token or a URL with a password inside an innocent key is caught too.

use serde_json::{Map, Value};

use super::values::Values;
use crate::redact;

/// What a masked value is replaced by.
pub const MASK: &str = "********";

/// Words that make a values key credential-looking, anywhere in it (`adminPassword`,
/// `auth.secretKey`, `tlsPrivateKey`). Deliberately broad: a masked name is a click away, a shown
/// credential cannot be taken back.
const WORDS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "credential",
    "privatekey",
    "private_key",
    "private-key",
    "accesskey",
    "access_key",
    "access-key",
    "authorization",
    "htpasswd",
];

/// Whether a values key reads like a credential.
pub fn is_sensitive_key(key: &str) -> bool {
    if redact::is_sensitive_field(key) {
        return true;
    }
    let lower = key.to_ascii_lowercase();
    WORDS.iter().any(|word| lower.contains(word))
}

/// `values` with the credential-looking entries masked, and how many values were masked.
///
/// * a scalar under a sensitive key (a non-empty string, a number, a boolean) becomes [`MASK`];
/// * a map or list under a sensitive key is masked leaf by leaf (its keys stay readable);
/// * every other string goes through the shared scrubber, which replaces secret-bearing text
///   (tokens, PEM blocks, URL credentials) inside it.
///
/// The tree's shape never changes, so the user sees which keys exist.
pub fn mask_values(values: &Values) -> (Values, usize) {
    let mut count = 0;
    let masked = mask_value(values.as_json(), false, &mut count);
    (Values::from_json(masked), count)
}

fn mask_value(value: &Value, sensitive: bool, count: &mut usize) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, inner)| {
                    (
                        key.clone(),
                        mask_value(inner, sensitive || is_sensitive_key(key), count),
                    )
                })
                .collect::<Map<_, _>>(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|inner| mask_value(inner, sensitive, count))
                .collect(),
        ),
        Value::String(text) if sensitive && !text.is_empty() => {
            *count += 1;
            Value::String(MASK.to_owned())
        }
        Value::String(text) => match redact::redact(text) {
            std::borrow::Cow::Borrowed(_) => value.clone(),
            std::borrow::Cow::Owned(clean) => {
                *count += 1;
                Value::String(clean)
            }
        },
        Value::Number(_) | Value::Bool(_) if sensitive => {
            *count += 1;
            Value::String(MASK.to_owned())
        }
        other => other.clone(),
    }
}

/// Notes text with secret-bearing substrings replaced (charts often print a generated password
/// there). Always applied: notes have no reveal.
pub fn mask_notes(notes: &str) -> String {
    redact::redact(notes).into_owned()
}
