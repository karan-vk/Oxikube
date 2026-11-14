//! [`ManifestError`].

/// Why an `extension.toml` was rejected. `Display` is shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    /// Not valid toml.
    #[error("extension.toml is not valid TOML: {0}")]
    Syntax(String),
    /// A field the schema does not know.
    #[error("unknown field `{field}` in extension.toml: {detail}")]
    UnknownField {
        /// The unknown field name.
        field: String,
        /// The toml error text (line, column, expected fields).
        detail: String,
    },
    /// A required field is absent.
    #[error("missing required field `{field}` in extension.toml: {detail}")]
    MissingField {
        /// The missing field name.
        field: String,
        /// The toml error text.
        detail: String,
    },
    /// A field has the wrong type or a value that breaks a rule.
    #[error("invalid `{field}` in extension.toml: {reason}")]
    InvalidValue {
        /// The field (or `table.field`) at fault; empty when toml did not say.
        field: String,
        /// What is wrong with it.
        reason: String,
    },
}

impl ManifestError {
    pub(super) fn invalid(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidValue {
            field: field.into(),
            reason: reason.into(),
        }
    }

    /// Classifies a serde error from toml by its message: toml reports unknown
    /// and missing fields as ``unknown field `x` ...`` / ``missing field `x` ``.
    pub(super) fn from_schema(error: toml::de::Error) -> Self {
        let detail = error.to_string();
        let message = error.message();
        if let Some(field) = quoted_after(message, "unknown field") {
            Self::UnknownField { field, detail }
        } else if let Some(field) = quoted_after(message, "missing field") {
            Self::MissingField { field, detail }
        } else {
            Self::InvalidValue {
                field: String::new(),
                reason: detail,
            }
        }
    }
}

/// The backtick-quoted word after `prefix`, when `message` starts with it.
fn quoted_after(message: &str, prefix: &str) -> Option<String> {
    let rest = message.strip_prefix(prefix)?;
    let rest = rest.trim_start().strip_prefix('`')?;
    Some(rest.split('`').next()?.to_owned())
}
