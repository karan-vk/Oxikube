//! [`Refusal`]: why the `resource::CopyTo` handler turned a copy down before writing, attached to
//! its error so the service can tell the cases apart without parsing messages.

use std::error::Error as StdError;

use oxikube_domain::OxiError;

/// A precondition of a copy that the target does not meet.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Refusal {
    /// The target does not serve the kind.
    #[error("{0}")]
    KindNotServed(String),
    /// The target serves the kind in other versions only.
    #[error("{0}")]
    VersionNotServed(String),
    /// The target namespace does not exist.
    #[error("namespace {0} does not exist on the target")]
    NamespaceMissing(String),
    /// The source object could not be read.
    #[error("{0}")]
    SourceUnavailable(String),
    /// The target name is not a valid DNS-1123 name.
    #[error("{0}")]
    InvalidName(String),
}

impl Refusal {
    /// The error for this refusal: a `Validation` error (the copy is wrong as asked) carrying the
    /// refusal as its source.
    pub(crate) fn into_error(self) -> OxiError {
        OxiError::validation(self.to_string()).with_source(self)
    }

    /// The refusal `error` carries, if it was made by [`into_error`](Self::into_error).
    pub(crate) fn of(error: &OxiError) -> Option<&Refusal> {
        StdError::source(error)?.downcast_ref::<Refusal>()
    }
}

/// Whether `name` is a DNS-1123 subdomain (lowercase alphanumerics, `-` and `.`, starting and
/// ending with an alphanumeric, at most 253 characters). A Namespace or Service name is the
/// stricter label (no dots, at most 63), checked for those two kinds.
///
/// # Errors
///
/// The sentence that says what is wrong.
pub(crate) fn validate_name(name: &str, label: bool) -> Result<(), String> {
    let (limit, what) = if label {
        (63, "DNS-1123 label")
    } else {
        (253, "DNS-1123 subdomain")
    };
    let alphanumeric = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    let valid_char = |c: char| alphanumeric(c) || c == '-' || (!label && c == '.');
    if name.is_empty() {
        return Err("the name is empty".to_owned());
    }
    if name.len() > limit {
        return Err(format!("the name is longer than {limit} characters"));
    }
    let edges = name.chars().next().is_some_and(alphanumeric)
        && name.chars().next_back().is_some_and(alphanumeric);
    if !name.chars().all(valid_char) || !edges {
        return Err(format!(
            "{name:?} is not a valid {what}: use lowercase letters, digits and '-'{}, starting \
             and ending with a letter or digit",
            if label { "" } else { " and '.'" }
        ));
    }
    Ok(())
}
