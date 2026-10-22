//! [`ReleaseFilter`]: the table's `/` filter over release rows (E07-S04 grammar).

use oxikube_domain::helm::Release;

use crate::store::filter::{NameFilter, parse};

/// Why a filter does not apply to releases. Shown next to the filter field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterProblem {
    /// The text does not parse (an invalid regular expression, for example).
    Invalid(String),
    /// A part of the grammar that has no meaning for releases (`-l`, `-s`).
    Unsupported(&'static str),
}

impl std::fmt::Display for FilterProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(why) => f.write_str(why),
            Self::Unsupported(why) => f.write_str(why),
        }
    }
}

/// A parsed filter over releases: the resource table's grammar (`foo`, `/foo`, a regular
/// expression, `!foo`, `-f fuzzy`), matched against the release name, namespace, chart name and
/// status. A release passes a plain filter when any of those matches; it passes an inverted one
/// (`!foo`) when none does. A filter that does not parse lets everything through and says why
/// ([`problem`](Self::problem)).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReleaseFilter {
    pattern: Option<NameFilter>,
    problem: Option<FilterProblem>,
}

impl ReleaseFilter {
    /// Parses `text`. Compiles the pattern once; call it per edit, not per row.
    pub fn parse(text: &str) -> Self {
        let expr = match parse(text) {
            Ok(expr) => expr,
            Err(error) => {
                return Self {
                    pattern: None,
                    problem: Some(FilterProblem::Invalid(error.to_string())),
                };
            }
        };
        let parts = expr.parts();
        let problem = if parts.selector.is_some() {
            Some(FilterProblem::Unsupported(
                "label selectors (-l) do not apply to releases",
            ))
        } else if parts.filter.status.is_some() {
            Some(FilterProblem::Unsupported(
                "status buckets (-s) do not apply to releases; type the status instead",
            ))
        } else {
            None
        };
        Self {
            pattern: parts.filter.pattern,
            problem,
        }
    }

    /// Why the filter does not apply, if it does not.
    pub fn problem(&self) -> Option<&FilterProblem> {
        self.problem.as_ref()
    }

    /// Whether the filter lets everything through.
    pub fn is_empty(&self) -> bool {
        self.pattern.is_none()
    }

    /// Whether `release` passes.
    pub fn matches(&self, release: &Release) -> bool {
        let Some(pattern) = &self.pattern else {
            return true;
        };
        let fields = [
            release.release.name.as_str(),
            release.release.namespace.as_str(),
            release.chart.name.as_str(),
            release.status.as_str(),
        ];
        if pattern.is_inverse() {
            fields.iter().all(|field| pattern.matches(field))
        } else {
            fields.iter().any(|field| pattern.matches(field))
        }
    }

    /// The rows of `rows` that pass, in order.
    pub fn apply(&self, rows: &[Release]) -> Vec<usize> {
        rows.iter()
            .enumerate()
            .filter(|(_, release)| self.matches(release))
            .map(|(ix, _)| ix)
            .collect()
    }
}
