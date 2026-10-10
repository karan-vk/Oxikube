//! [`FindQuery`] and the scans that turn a text into match positions.

use std::ops::Range;
use std::sync::Arc;

use regex::{Regex, RegexBuilder};

use crate::search::filter::{FilterError, MAX_FILTER_LEN, TextPattern};

/// The most matches a scan reports; a text with more is cut here and the result says so. A
/// 10 MB text of one repeated letter must not allocate a match per character.
pub const MAX_MATCHES: usize = 10_000;

/// What to search for: a pattern compiled once per edit. Plain text matches as a substring, text
/// with regex syntax as a regular expression, always ignoring case (the `/` filter's rule).
#[derive(Debug, Clone)]
pub struct FindQuery {
    regex: Arc<Regex>,
}

impl FindQuery {
    /// Compiles `text`. `None` for empty text (it finds nothing, instead of everything).
    ///
    /// # Errors
    ///
    /// [`FilterError::Regex`] when the pattern has regex syntax that does not compile, or
    /// [`FilterError::TooLong`] past [`MAX_FILTER_LEN`] characters.
    pub fn new(text: &str) -> Result<Option<Self>, FilterError> {
        if text.is_empty() {
            return Ok(None);
        }
        if text.chars().count() > MAX_FILTER_LEN {
            return Err(FilterError::TooLong(MAX_FILTER_LEN));
        }
        let regex = match TextPattern::compile(text).map_err(FilterError::Regex)? {
            TextPattern::Regex { regex, .. } => regex,
            TextPattern::Substring(literal) => RegexBuilder::new(&regex::escape(&literal))
                .case_insensitive(true)
                .build()
                .map(Arc::new)
                .map_err(|error| FilterError::Regex(error.to_string()))?,
        };
        Ok(Some(Self { regex }))
    }

    /// The byte ranges of the matches in `line`, in order, appended to `out`.
    pub fn ranges_in(&self, line: &str, out: &mut Vec<Range<usize>>) {
        out.extend(
            self.regex
                .find_iter(line)
                .filter(|m| !m.is_empty())
                .map(|m| m.range()),
        );
    }
}

/// The matches of a scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindMatches {
    /// The byte ranges of the matches in the whole text, in order.
    pub ranges: Vec<Range<usize>>,
    /// Whether the scan stopped at [`MAX_MATCHES`].
    pub truncated: bool,
}

impl FindMatches {
    /// How many matches were found.
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// Whether nothing matched.
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }
}

/// Finds `query` in `text` as a whole: the byte ranges in the text, at most [`MAX_MATCHES`].
/// A match does not span lines (a pattern with `\n` finds nothing), so a text is scanned line by
/// line without copying it.
pub fn find_in_text(text: &str, query: &FindQuery) -> FindMatches {
    let mut found = FindMatches::default();
    let mut base = 0;
    let mut scratch = Vec::new();
    for line in text.split('\n') {
        scratch.clear();
        query.ranges_in(line, &mut scratch);
        for range in scratch.drain(..) {
            if found.ranges.len() == MAX_MATCHES {
                found.truncated = true;
                return found;
            }
            found.ranges.push(base + range.start..base + range.end);
        }
        base += line.len() + 1;
    }
    found
}
