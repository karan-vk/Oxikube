//! The filter grammar: [`parse`] turns what the user typed into a [`FilterExpr`].
//!
//! ```text
//! input    = [ "/" ] ( "" | "!" body | body )
//! body     = "-l" selector            label selector (server-side)
//!          | "-f" text                fuzzy name match
//!          | text                     name regex, or substring when it has no regex syntax
//! ```
//!
//! This is k9s's grammar (`/regex`, `/!regex`, `/-l k=v`, `/-f text`). One leading `/` is
//! accepted and ignored, so a filter pasted from k9s works. Names never start with `-` or
//! contain whitespace, so `-l` and `-f` cannot clash with a name; to match a name by a pattern
//! that starts with `-` or `!`, escape it (`\-x`, `\!x`). A bare `!` or `-` is a filter being
//! typed and means no filter.

use thiserror::Error;

use super::fuzzy::Fuzzy;
use super::name::TextPattern;
use crate::store::{LabelSelector, SelectorError};

/// A parsed filter. Patterns are compiled; building one is the once-per-edit cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterExpr {
    /// No filter.
    Empty,
    /// `/text`: names matching a regex or substring, case-insensitive.
    Text(TextPattern),
    /// `/!text` or `/!-f text`: the names the inner filter does not match. [`parse`] only makes
    /// this over `Text` and `Fuzzy`.
    Inverse(Box<FilterExpr>),
    /// `/-l selector`: a label selector, applied by the server.
    LabelSelector(LabelSelector),
    /// `/-f text`: names containing the characters in order, ranked by closeness.
    Fuzzy(Fuzzy),
}

/// Why an input is not a filter. The messages are shown in the filter bar.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FilterError {
    /// The pattern is not a valid regular expression.
    #[error("invalid regex: {0}")]
    Regex(String),
    /// The label selector does not parse.
    #[error("{0}")]
    Selector(SelectorError),
    /// A `-x` flag other than `-l` and `-f`.
    #[error("unknown option `{0}` (use -l for labels, -f for fuzzy)")]
    UnknownFlag(String),
    /// `!` in front of `-l`.
    #[error("a label selector cannot be inverted (use != or notin)")]
    InvertedSelector,
}

/// Parses `input` as the text of the filter bar. See the [`filter`](super) module for the grammar.
///
/// # Errors
///
/// [`FilterError`] when the regex or the selector is malformed or the flag is unknown. The
/// caller keeps showing the rows of the last good filter.
pub fn parse(input: &str) -> Result<FilterExpr, FilterError> {
    let text = input.trim();
    let text = text.strip_prefix('/').unwrap_or(text).trim_start();
    match text.strip_prefix('!') {
        Some(rest) => parse_inverse(rest.trim_start()),
        None => parse_body(text),
    }
}

fn parse_inverse(rest: &str) -> Result<FilterExpr, FilterError> {
    if rest.is_empty() {
        return Ok(FilterExpr::Empty);
    }
    match parse_body(rest)? {
        FilterExpr::Empty => Ok(FilterExpr::Empty),
        // `!-f` with no text is a filter being typed, not "exclude everything".
        FilterExpr::Fuzzy(fuzzy) if fuzzy.is_empty() => Ok(FilterExpr::Empty),
        FilterExpr::LabelSelector(_) => Err(FilterError::InvertedSelector),
        inner => Ok(FilterExpr::Inverse(Box::new(inner))),
    }
}

fn parse_body(text: &str) -> Result<FilterExpr, FilterError> {
    if text.is_empty() || text == "-" {
        return Ok(FilterExpr::Empty);
    }
    if text.starts_with('-') {
        let (flag, rest) = text
            .split_once(char::is_whitespace)
            .map_or((text, ""), |(flag, rest)| (flag, rest.trim()));
        return match flag {
            "-l" => LabelSelector::parse(rest)
                .map(FilterExpr::LabelSelector)
                .map_err(FilterError::Selector),
            "-f" => Ok(FilterExpr::Fuzzy(Fuzzy::new(rest))),
            other => Err(FilterError::UnknownFlag(other.to_owned())),
        };
    }
    TextPattern::compile(text)
        .map(FilterExpr::Text)
        .map_err(FilterError::Regex)
}
