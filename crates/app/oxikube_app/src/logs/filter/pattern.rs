//! [`LogFilter`] (what the user typed and toggled) and [`LogMatcher`] (it, compiled).

use std::ops::Range;

use regex::{Regex, RegexBuilder};

/// Compiled programs larger than this are refused (an accidental `(a{1000}){1000}` must not eat
/// memory). The regex crate matches in linear time, so a pattern cannot stall the viewer.
const SIZE_LIMIT: usize = 1 << 20;

/// A search or filter over log lines: a regular expression, whether case matters, and whether the
/// result is inverted. Plain data: it is what a session remembers and what an agent passes.
///
/// A pattern without regex syntax is a literal, so `error` finds `error`. The empty pattern is
/// no filter: every line matches.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LogFilter {
    /// The regular expression (the `regex` crate's syntax).
    pub pattern: String,
    /// Whether upper and lower case differ. Off by default, like `grep -i`.
    pub case_sensitive: bool,
    /// Match the lines that do *not* contain the pattern (k9s's `!`).
    pub inverse: bool,
}

impl LogFilter {
    /// A case-insensitive, non-inverse filter for `pattern`.
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            ..Self::default()
        }
    }

    /// Whether the filter does nothing (empty pattern): every line matches, nothing is
    /// highlighted.
    pub fn is_empty(&self) -> bool {
        self.pattern.is_empty()
    }

    /// Compiles the pattern, once per edit.
    ///
    /// # Errors
    ///
    /// [`FilterError`] when the pattern is not a valid (or is too large a) regular expression.
    pub fn compile(&self) -> Result<LogMatcher, FilterError> {
        let regex = if self.pattern.is_empty() {
            None
        } else {
            let regex = RegexBuilder::new(&self.pattern)
                .case_insensitive(!self.case_sensitive)
                .size_limit(SIZE_LIMIT)
                .build()
                .map_err(|error| FilterError::new(&error))?;
            Some(regex)
        };
        Ok(LogMatcher {
            filter: self.clone(),
            regex,
        })
    }
}

/// Why a pattern did not compile, in one line for the search bar.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct FilterError {
    message: String,
}

impl FilterError {
    fn new(error: &regex::Error) -> Self {
        // The regex crate's `Display` is a multi-line report with the pattern echoed back; the
        // bar has room for the reason only (`unclosed group`).
        let text = error.to_string();
        let message = text
            .lines()
            .rev()
            .find_map(|line| line.trim().strip_prefix("error: "))
            .unwrap_or_else(|| text.lines().next().unwrap_or("invalid pattern"))
            .to_owned();
        Self { message }
    }

    /// The reason, e.g. `unclosed group`.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// A [`LogFilter`] compiled to a regex. Cheap to share (wrap it in an `Arc`); matching allocates
/// nothing.
#[derive(Clone, Debug)]
pub struct LogMatcher {
    filter: LogFilter,
    regex: Option<Regex>,
}

impl LogMatcher {
    /// The filter this was compiled from.
    pub fn filter(&self) -> &LogFilter {
        &self.filter
    }

    /// Whether the filter does anything (a non-empty pattern).
    pub fn is_active(&self) -> bool {
        self.regex.is_some()
    }

    /// Whether `text` passes the filter: it contains the pattern, or (inverse) it does not.
    /// Every line passes the empty filter. This is the predicate the viewer and the agent's
    /// `grep` share.
    pub fn matches(&self, text: &str) -> bool {
        match &self.regex {
            None => true,
            Some(regex) => regex.is_match(text) != self.filter.inverse,
        }
    }

    /// The byte ranges of the pattern's occurrences in `text` (never empty ranges), at most
    /// `limit` of them: what the viewer highlights. Empty for the empty pattern.
    pub fn spans(&self, text: &str, limit: usize) -> Vec<Range<usize>> {
        let Some(regex) = &self.regex else {
            return Vec::new();
        };
        regex
            .find_iter(text)
            .filter(|found| !found.is_empty())
            .take(limit)
            .map(|found| found.range())
            .collect()
    }
}
