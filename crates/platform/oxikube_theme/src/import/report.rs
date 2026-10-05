//! What the importer found wrong with a theme file, without failing the whole import.

use std::collections::BTreeSet;
use std::fmt;

/// The theme file could not be read as a theme family at all.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ImportError {
    /// Not valid JSON (comments and trailing commas are allowed).
    #[error("theme file is not valid JSON: {0}")]
    Json(String),
    /// Valid JSON, but not an object with a `themes` array.
    #[error("theme file must be an object with a `themes` array")]
    NotAThemeFamily,
}

/// One problem in a theme file. The affected key keeps the fallback value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportDiagnostic {
    /// A colour value that is not `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
    InvalidColor {
        /// The theme the key is in.
        theme: String,
        /// The offending key (dotted, e.g. `editor.background`).
        key: String,
        /// The value as written.
        value: String,
        /// Why it did not parse.
        message: String,
    },
    /// A value of the wrong JSON type (a number where a colour string belongs, ...).
    InvalidValue {
        /// The theme the key is in.
        theme: String,
        /// The offending key.
        key: String,
        /// What was expected there.
        expected: &'static str,
    },
    /// A theme entry that was left out (no name, or an appearance that is not light/dark).
    SkippedTheme {
        /// Position in the family's `themes` array.
        index: usize,
        /// Why.
        reason: String,
    },
    /// The file declares a schema other than v0.2.0; it is still read, best effort.
    SchemaVersion {
        /// The `$schema` value.
        found: String,
    },
}

impl fmt::Display for ImportDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidColor {
                theme,
                key,
                value,
                message,
            } => write!(f, "{theme}: `{key}` is not a colour ({value:?}): {message}"),
            Self::InvalidValue {
                theme,
                key,
                expected,
            } => write!(f, "{theme}: `{key}` should be {expected}"),
            Self::SkippedTheme { index, reason } => {
                write!(f, "theme #{index} skipped: {reason}")
            }
            Self::SchemaVersion { found } => {
                write!(f, "unsupported theme schema {found:?}; reading as v0.2.0")
            }
        }
    }
}

/// Everything noticed while importing one family file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Problems that left a key at its fallback value, in file order.
    pub diagnostics: Vec<ImportDiagnostic>,
    /// Keys no mapping exists for (ignored, logged at debug level), across all themes.
    pub unknown_keys: BTreeSet<String>,
}

impl ImportReport {
    /// Whether the file imported without any problem (unknown keys are not problems).
    pub fn is_clean(&self) -> bool {
        self.diagnostics.is_empty()
    }
}
