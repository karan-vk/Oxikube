//! The filter grammar: [`parse`] turns what the user typed into a [`FilterExpr`].
//!
//! ```text
//! input    = [ "/" ] ( "" | "!" body | body ) [ "-l" selector ]
//! body     = "-l" selector            label selector (server-side)
//!          | "-f" text                fuzzy name match
//!          | text                     name regex, or substring when it has no regex syntax
//! ```
//!
//! A name filter may be followed by `-l selector` (`api -l app=x`): both apply. That is how the
//! `:` jump bar's `:pod /api app=x` fills the bar.
//!
//! This is k9s's grammar (`/regex`, `/!regex`, `/-l k=v`, `/-f text`). One leading `/` is
//! accepted and ignored, so a filter pasted from k9s works. Names never start with `-` or
//! contain whitespace, so `-l` and `-f` cannot clash with a name; to match a name by a pattern
//! that starts with `-` or `!`, escape it (`\-x`, `\!x`). A bare `!` or `-` is a filter being
//! typed and means no filter. A bare `!` (or `!-f`) is different: it inverts nothing, so it is
//! an error that keeps the last good filter (`/!` alone never means "hide every row").
//!
//! The whole input is capped at [`MAX_FILTER_LEN`] characters: the pattern is recompiled on every
//! keystroke, and a name filter never needs more.

use thiserror::Error;

use super::fuzzy::Fuzzy;
use super::name::TextPattern;
use crate::store::{LabelSelector, SelectorError};

/// The longest filter (in characters, `/` and `-l` selector included) [`parse`] accepts. A longer
/// one is an error, so a pasted blob never reaches the regex compiler on the typing path.
pub const MAX_FILTER_LEN: usize = 512;

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
    /// A name filter and a label selector together: `api -l app=x`. `name` is never a
    /// `LabelSelector` or another `Narrowed`.
    Narrowed {
        /// The name filter (`Text`, `Inverse` or `Fuzzy`).
        name: Box<FilterExpr>,
        /// The selector the server applies.
        selector: LabelSelector,
    },
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
    /// `!` with no pattern after it.
    #[error("`!` needs a pattern to invert (`!web`)")]
    NothingToInvert,
    /// The input is longer than [`MAX_FILTER_LEN`].
    #[error("the filter is too long ({0} characters at most)")]
    TooLong(usize),
}

/// Parses `input` as the text of the filter bar. See the [`filter`](super) module for the grammar.
///
/// # Errors
///
/// [`FilterError`] when the regex or the selector is malformed, the flag is unknown, the input
/// is longer than [`MAX_FILTER_LEN`] or a `!` has nothing to invert. The caller keeps showing the
/// rows of the last good filter.
pub fn parse(input: &str) -> Result<FilterExpr, FilterError> {
    let text = input.trim();
    // Bytes first (cheap): a text this long in bytes is only long in characters if it is ASCII.
    if text.len() > MAX_FILTER_LEN && text.chars().count() > MAX_FILTER_LEN {
        return Err(FilterError::TooLong(MAX_FILTER_LEN));
    }
    let text = text.strip_prefix('/').unwrap_or(text).trim_start();
    match split_selector(text) {
        Some((name, selector)) => {
            let selector = LabelSelector::parse(selector).map_err(FilterError::Selector)?;
            let name = parse_name(name)?;
            Ok(match name {
                FilterExpr::Empty => FilterExpr::LabelSelector(selector),
                name if selector.is_empty() => name,
                name => FilterExpr::Narrowed {
                    name: Box::new(name),
                    selector,
                },
            })
        }
        None => parse_name_or_selector(text),
    }
}

/// Splits `name -l selector` at the `-l` word that follows a name. `None` when `-l` is the first
/// word (a plain selector filter) or absent.
fn split_selector(text: &str) -> Option<(&str, &str)> {
    let mut offset = 0;
    for word in text.split_inclusive(char::is_whitespace) {
        let at = offset;
        offset += word.len();
        if at > 0 && word.trim_end() == "-l" {
            return Some((text[..at].trim_end(), text[offset..].trim()));
        }
    }
    None
}

fn parse_name_or_selector(text: &str) -> Result<FilterExpr, FilterError> {
    match text.strip_prefix('!') {
        Some(rest) => parse_inverse(rest.trim_start()),
        None => parse_body(text),
    }
}

/// The name part of `name -l selector`: a name filter, never a selector.
fn parse_name(text: &str) -> Result<FilterExpr, FilterError> {
    match parse_name_or_selector(text)? {
        FilterExpr::LabelSelector(_) => Err(FilterError::UnknownFlag("-l".to_owned())),
        name => Ok(name),
    }
}

fn parse_inverse(rest: &str) -> Result<FilterExpr, FilterError> {
    if rest.is_empty() {
        return Err(FilterError::NothingToInvert);
    }
    match parse_body(rest)? {
        // `!-` / `!-f` with no text: nothing to invert, and never "exclude everything".
        FilterExpr::Empty => Err(FilterError::NothingToInvert),
        FilterExpr::Fuzzy(fuzzy) if fuzzy.is_empty() => Err(FilterError::NothingToInvert),
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
