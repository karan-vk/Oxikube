//! [`SubjectFilter`]: the table's `/` filter over subject and matrix rows (E07-S04 grammar).

use crate::search::filter::{NameFilter, parse};

/// A parsed `/` filter over text rows: `foo`, `/foo`, a regular expression, `!foo`, `-f fuzzy`.
/// A row passes a plain filter when any of its fields matches and an inverted one when none does.
/// A filter that does not parse lets everything through and says why ([`problem`](Self::problem)).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubjectFilter {
    pattern: Option<NameFilter>,
    problem: Option<String>,
}

impl SubjectFilter {
    /// Parses `text`. Compiles the pattern once; call it per edit, not per row.
    pub fn parse(text: &str) -> Self {
        let expr = match parse(text) {
            Ok(expr) => expr,
            Err(error) => {
                return Self {
                    pattern: None,
                    problem: Some(error.to_string()),
                };
            }
        };
        let parts = expr.parts();
        let problem = if parts.selector.is_some() {
            Some("label selectors (-l) do not apply here".to_owned())
        } else if parts.filter.status.is_some() {
            Some("status buckets (-s) do not apply here".to_owned())
        } else {
            None
        };
        Self {
            pattern: parts.filter.pattern,
            problem,
        }
    }

    /// Why the filter does not apply, if it does not.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Whether the filter lets everything through.
    pub fn is_empty(&self) -> bool {
        self.pattern.is_none()
    }

    /// Whether a row with these `fields` passes.
    pub fn matches(&self, fields: &[&str]) -> bool {
        let Some(pattern) = &self.pattern else {
            return true;
        };
        if pattern.is_inverse() {
            fields.iter().all(|field| pattern.matches(field))
        } else {
            fields.iter().any(|field| pattern.matches(field))
        }
    }
}
