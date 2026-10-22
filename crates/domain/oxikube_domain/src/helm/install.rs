//! The install tab's vocabulary (E16-S09): name rules, the values text carried by `helm::Install`,
//! and the per-kind summary of a rendered manifest.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::error::HelmError;
use super::values::Values;

/// The longest release name Helm accepts (it appends suffixes to build object names).
pub const MAX_RELEASE_NAME: usize = 53;

/// The longest namespace name (a DNS-1123 label).
pub const MAX_NAMESPACE_NAME: usize = 63;

/// Whether `name` is a DNS-1123 label of at most `max` characters: lowercase letters, digits and
/// `-`, starting and ending with a letter or digit.
fn dns_label(name: &str, max: usize) -> Result<(), String> {
    if name.is_empty() {
        return Err("Enter a name.".to_owned());
    }
    if name.len() > max {
        return Err(format!("At most {max} characters ({} now).", name.len()));
    }
    let valid_char = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
    if !name.chars().all(valid_char) {
        return Err("Use lowercase letters, digits and '-' only.".to_owned());
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err("Start and end with a letter or a digit.".to_owned());
    }
    Ok(())
}

/// Checks a release name the way Helm does: a DNS-1123 label of at most [`MAX_RELEASE_NAME`]
/// characters. The error is a sentence for the form.
///
/// # Errors
///
/// What is wrong with `name`.
pub fn validate_release_name(name: &str) -> Result<(), String> {
    dns_label(name, MAX_RELEASE_NAME)
}

/// Checks a namespace name: a DNS-1123 label of at most [`MAX_NAMESPACE_NAME`] characters.
///
/// # Errors
///
/// What is wrong with `name`.
pub fn validate_namespace_name(name: &str) -> Result<(), String> {
    dns_label(name, MAX_NAMESPACE_NAME)
}

/// The values YAML a `helm::Install` carries, as the user wrote it.
///
/// Values can hold credentials, so `Debug` never prints the text (a derived `Debug` of a command
/// stays safe) and nothing persists it: it lives in the command for the length of a dispatch and
/// reaches `helm` on its stdin. Serialised as a plain string (an agent's tool call can supply it).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ValuesYaml(String);

impl ValuesYaml {
    /// Wraps `text`.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The text. Calling this is the audit point for "does this leak?".
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// `true` when there is nothing but whitespace (the chart's defaults apply).
    pub fn is_blank(&self) -> bool {
        self.0.trim().is_empty()
    }

    /// The values tree.
    ///
    /// # Errors
    ///
    /// [`HelmError::Parse`] when the text is not valid YAML or is not a map.
    pub fn parse(&self) -> Result<Values, HelmError> {
        let values = Values::from_yaml(&self.0)?;
        if !values.as_json().is_object() {
            return Err(HelmError::Parse(
                "values must be a map of keys (`key: value`)".to_owned(),
            ));
        }
        Ok(values)
    }
}

impl fmt::Debug for ValuesYaml {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ValuesYaml({} bytes, redacted)", self.0.len())
    }
}

/// How many objects of each kind a rendered manifest holds, for the preview's header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestSummary {
    /// Kind to count, by kind name.
    pub kinds: BTreeMap<String, usize>,
}

impl ManifestSummary {
    /// Counts the documents of `manifest` (YAML separated by `---`) by their top-level `kind:`.
    /// Lexical and linear: no YAML parse, so a multi-megabyte manifest costs one scan. Empty
    /// documents and documents without a kind are skipped.
    pub fn of(manifest: &str) -> Self {
        let mut kinds = BTreeMap::new();
        let mut kind: Option<&str> = None;
        let mut flush = |kind: &mut Option<&str>| {
            if let Some(k) = kind.take() {
                *kinds.entry(k.to_owned()).or_insert(0) += 1;
            }
        };
        for line in manifest.lines() {
            if line == "---" || line.starts_with("--- ") {
                flush(&mut kind);
            } else if kind.is_none()
                && let Some(rest) = line.strip_prefix("kind:")
            {
                let value = rest.split('#').next().unwrap_or("").trim();
                let value = value.trim_matches(|c| c == '"' || c == '\'');
                if !value.is_empty() {
                    kind = Some(value);
                }
            }
        }
        flush(&mut kind);
        Self { kinds }
    }

    /// The number of objects.
    pub fn total(&self) -> usize {
        self.kinds.values().sum()
    }

    /// `3 objects: 1 Deployment, 1 Service, 1 ConfigMap`-style text, kinds by count then name.
    pub fn describe(&self) -> String {
        let total = self.total();
        if total == 0 {
            return "No objects".to_owned();
        }
        let mut kinds: Vec<_> = self.kinds.iter().collect();
        kinds.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        let parts: Vec<String> = kinds.iter().map(|(k, n)| format!("{n} {k}")).collect();
        let noun = if total == 1 { "object" } else { "objects" };
        format!("{total} {noun}: {}", parts.join(", "))
    }
}
