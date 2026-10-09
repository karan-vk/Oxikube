//! [`ResourceError`]: why a JSON value is not a [`Resource`](super::Resource).

use crate::error::OxiError;

/// Why a JSON value could not become a [`Resource`], or a [`Resource`] could not
/// be rendered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResourceError {
    /// The input was not a JSON object.
    #[error("resource must be a JSON object")]
    NotAnObject,
    /// A required top-level field (`apiVersion`, `kind`, `metadata`) is absent.
    #[error("resource is missing `{field}`")]
    MissingField {
        /// JSON path of the absent field.
        field: &'static str,
    },
    /// `metadata.name` is absent or empty.
    #[error("resource is missing `metadata.name`")]
    MissingName,
    /// A field is present but has the wrong type or an unparsable value.
    #[error("invalid `{field}`: {reason}")]
    InvalidField {
        /// JSON path of the offending field.
        field: &'static str,
        /// What was wrong with it.
        reason: String,
    },
    /// YAML serialisation failed.
    #[error("yaml serialisation failed: {0}")]
    Yaml(String),
}

impl From<ResourceError> for OxiError {
    fn from(err: ResourceError) -> Self {
        OxiError::validation(err.to_string())
    }
}
