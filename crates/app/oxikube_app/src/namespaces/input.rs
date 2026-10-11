//! What the create dialog and `namespace::Create` accept: a DNS-label name and optional labels.

use std::collections::BTreeMap;

use oxikube_domain::{OxiError, OxiResult};
use serde_json::{Value, json};

use crate::session::namespaces::is_valid_namespace_name;

/// Why a label was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LabelError {
    /// A term is not `key=value`.
    #[error("`{0}` is not key=value")]
    NotKeyValue(String),
    /// A key is not a Kubernetes label key.
    #[error("`{0}` is not a valid label key")]
    Key(String),
    /// A value is not a Kubernetes label value.
    #[error("`{value}` is not a valid value for label `{key}`")]
    Value {
        /// The label.
        key: String,
        /// The refused value.
        value: String,
    },
}

/// Parses `key=value` terms separated by commas or whitespace (`team=a, env=dev`) into labels.
/// Blank input is no labels. Every key and value is checked with [`validate_labels`].
///
/// # Errors
///
/// The first term that is not a valid label.
pub fn parse_labels(text: &str) -> Result<BTreeMap<String, String>, LabelError> {
    let mut labels = BTreeMap::new();
    for term in text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
    {
        let (key, value) = term
            .split_once('=')
            .ok_or_else(|| LabelError::NotKeyValue(term.to_owned()))?;
        labels.insert(key.to_owned(), value.to_owned());
    }
    validate_labels(&labels)?;
    Ok(labels)
}

/// Checks `labels` against Kubernetes' syntax: a key is an optional DNS-subdomain prefix, a
/// slash, and a name of at most 63 alphanumerics, `-`, `_` or `.` that starts and ends
/// alphanumeric; a value is empty or the same shape as a name.
///
/// # Errors
///
/// The first label that breaks the syntax.
pub fn validate_labels(labels: &BTreeMap<String, String>) -> Result<(), LabelError> {
    for (key, value) in labels {
        if !valid_key(key) {
            return Err(LabelError::Key(key.clone()));
        }
        if !value.is_empty() && !valid_name_part(value) {
            return Err(LabelError::Value {
                key: key.clone(),
                value: value.clone(),
            });
        }
    }
    Ok(())
}

fn valid_key(key: &str) -> bool {
    match key.split_once('/') {
        Some((prefix, name)) => {
            !prefix.is_empty()
                && prefix.len() <= 253
                && prefix.split('.').all(is_valid_namespace_name)
                && valid_name_part(name)
        }
        None => valid_name_part(key),
    }
}

/// Up to 63 characters: alphanumerics, `-`, `_`, `.`; alphanumeric at both ends.
fn valid_name_part(text: &str) -> bool {
    let bytes = text.as_bytes();
    let edge = |b: u8| b.is_ascii_alphanumeric();
    !bytes.is_empty()
        && bytes.len() <= 63
        && edge(bytes[0])
        && edge(bytes[bytes.len() - 1])
        && bytes
            .iter()
            .all(|&b| edge(b) || matches!(b, b'-' | b'_' | b'.'))
}

/// Checks a namespace name: a DNS label.
///
/// # Errors
///
/// `Validation`, with the rule in words.
pub fn validate_name(name: &str) -> OxiResult<()> {
    if is_valid_namespace_name(name) {
        Ok(())
    } else {
        Err(OxiError::validation(format!(
            "`{name}` is not a valid namespace name: use lowercase letters, digits and `-`, \
             starting and ending with a letter or digit, at most 63 characters"
        )))
    }
}

/// The Namespace manifest of `name` with `labels`.
pub fn manifest(name: &str, labels: &BTreeMap<String, String>) -> Value {
    let mut metadata = json!({ "name": name });
    if !labels.is_empty() {
        metadata["labels"] = json!(labels);
    }
    json!({ "apiVersion": "v1", "kind": "Namespace", "metadata": metadata })
}
