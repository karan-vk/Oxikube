//! The findings of the schema validator.

use std::fmt;
use std::ops::Range;

use crate::yaml::JsonPath;

/// How serious a [`Diagnostic`] is. The editor draws errors and warnings with different squiggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// The API server will reject the manifest (syntax, type, enum, pattern, required).
    Error,
    /// Probably a mistake the server may still accept (unknown field: CRDs and newer servers add
    /// fields; a duplicate list key).
    Warning,
}

impl Severity {
    /// `"error"` or `"warning"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// Which rule produced a [`Diagnostic`]. The [`as_str`](Self::as_str) spellings are stable: the
/// apply flow's error-to-field mapper (E10-S08) and the tests match on them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticCode {
    /// Broken YAML (from the spanned model, not the schema). `syntax`.
    Syntax,
    /// A mapping key the schema does not know. `unknown-field`.
    UnknownField,
    /// A value of the wrong YAML type, typed by the YAML 1.2 core schema. `type-mismatch`.
    TypeMismatch,
    /// A value outside the schema's `enum`. `enum`.
    Enum,
    /// A required property is missing. `required`.
    Required,
    /// A string that does not match the schema's `pattern`. `pattern`.
    Pattern,
    /// Two items of an `x-kubernetes-list-type: map` list share their list-map keys. `duplicate-key`.
    DuplicateKey,
    /// Two items of an `x-kubernetes-list-type: set` list are equal. `duplicate-item`.
    DuplicateItem,
}

impl DiagnosticCode {
    /// The stable code string (`unknown-field`, `type-mismatch`, `enum`, `required`, `pattern`,
    /// `syntax`, `duplicate-key`, `duplicate-item`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            DiagnosticCode::Syntax => "syntax",
            DiagnosticCode::UnknownField => "unknown-field",
            DiagnosticCode::TypeMismatch => "type-mismatch",
            DiagnosticCode::Enum => "enum",
            DiagnosticCode::Required => "required",
            DiagnosticCode::Pattern => "pattern",
            DiagnosticCode::DuplicateKey => "duplicate-key",
            DiagnosticCode::DuplicateItem => "duplicate-item",
        }
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One finding: a byte range of the buffer to underline, how bad it is and what is wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Zero-based index of the `---` document the finding is in.
    pub doc: usize,
    /// Bytes to underline, in the whole buffer (not relative to the document). The offending key
    /// (`unknown-field`, `duplicate-key`) or value; for `required` the key that owns the object
    /// (or its first key at the root and in a list item), since the missing key has no text.
    pub span: Range<usize>,
    /// Error or warning.
    pub severity: Severity,
    /// Human text: what was found and what was expected, with a "did you mean" where one fits.
    pub message: String,
    /// The rule, for programmatic matching.
    pub code: DiagnosticCode,
    /// Where in the document the finding is (the owning object's path for `required`), for
    /// hover and for mapping server errors onto the same fields.
    pub path: JsonPath,
}
