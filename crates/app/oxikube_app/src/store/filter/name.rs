//! The name predicates of the `/` filter: [`TextPattern`] (substring or regular expression),
//! [`NameMatcher`] (text or fuzzy) and [`NameFilter`] (a matcher, optionally inverted).
//!
//! Patterns are compiled once, when the filter is parsed, never per row. Matching is
//! case-insensitive.

use std::sync::Arc;

use regex::{Regex, RegexBuilder};

use super::fuzzy::Fuzzy;

/// Largest compiled regular expression accepted, in bytes (the regex crate's default is 10 MB;
/// a name filter never needs that much).
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// Characters that make a pattern a regular expression instead of a plain substring.
const META: &[char] = &[
    '\\', '.', '+', '*', '?', '(', ')', '|', '[', ']', '{', '}', '^', '$',
];

/// A `/text` pattern: a plain case-insensitive substring when it has no regular-expression
/// syntax (the common case, matched without the regex engine), else a case-insensitive regular
/// expression searched anywhere in the name.
#[derive(Debug, Clone)]
pub enum TextPattern {
    /// A literal, stored lower-cased.
    Substring(Arc<str>),
    /// A compiled regular expression and the text it came from.
    Regex {
        /// What the user typed.
        source: Arc<str>,
        /// The compiled expression.
        regex: Arc<Regex>,
    },
}

impl TextPattern {
    /// Compiles `text`.
    ///
    /// # Errors
    ///
    /// The regex engine's message (one line) when `text` has regular-expression syntax that does
    /// not compile.
    pub fn compile(text: &str) -> Result<Self, String> {
        if !text.contains(META) {
            return Ok(Self::Substring(text.to_lowercase().into()));
        }
        RegexBuilder::new(text)
            .case_insensitive(true)
            .size_limit(REGEX_SIZE_LIMIT)
            .build()
            .map(|regex| Self::Regex {
                source: text.into(),
                regex: Arc::new(regex),
            })
            .map_err(|e| one_line(&e))
    }

    /// Whether `name` matches.
    pub fn matches(&self, name: &str) -> bool {
        match self {
            Self::Substring(needle) => contains_ignore_case(name, needle),
            Self::Regex { regex, .. } => regex.is_match(name),
        }
    }

    /// The pattern as typed (lower-cased for a substring).
    pub fn as_str(&self) -> &str {
        match self {
            Self::Substring(text) => text,
            Self::Regex { source, .. } => source,
        }
    }

    /// Whether everything this pattern matches is also matched by `older`: both are substrings
    /// and `older`'s text is inside this one's. Regular expressions never narrow (appending to
    /// one can widen it: `a` to `a|b`).
    fn narrows(&self, older: &TextPattern) -> bool {
        match (self, older) {
            (Self::Substring(new), Self::Substring(old)) => new.contains(&**old),
            _ => false,
        }
    }
}

impl PartialEq for TextPattern {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Substring(a), Self::Substring(b)) => a == b,
            (Self::Regex { source: a, .. }, Self::Regex { source: b, .. }) => a == b,
            _ => false,
        }
    }
}

impl Eq for TextPattern {}

/// What a [`NameFilter`] matches names with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameMatcher {
    /// `/text`: a substring or regular expression.
    Text(TextPattern),
    /// `/-f text`: a subsequence, ranked by [`Fuzzy::score`].
    Fuzzy(Fuzzy),
}

/// A predicate on an object's name: a [`NameMatcher`] that is optionally inverted (`/!text`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameFilter {
    matcher: NameMatcher,
    inverse: bool,
}

impl NameFilter {
    /// `matcher`, matching the names it does.
    pub fn new(matcher: NameMatcher) -> Self {
        Self {
            matcher,
            inverse: false,
        }
    }

    /// The same matcher matching the names it does not.
    #[must_use]
    pub fn inverted(mut self) -> Self {
        self.inverse = !self.inverse;
        self
    }

    /// Whether this is the inverse (`/!`) form.
    pub fn is_inverse(&self) -> bool {
        self.inverse
    }

    /// Whether the filter lets everything through (an empty pattern, not inverted).
    pub fn is_empty(&self) -> bool {
        !self.inverse
            && match &self.matcher {
                NameMatcher::Text(text) => text.as_str().is_empty(),
                NameMatcher::Fuzzy(fuzzy) => fuzzy.is_empty(),
            }
    }

    /// Whether `name` passes.
    pub fn matches(&self, name: &str) -> bool {
        let hit = match &self.matcher {
            NameMatcher::Text(text) => text.matches(name),
            NameMatcher::Fuzzy(fuzzy) => fuzzy.matches(name),
        };
        hit != self.inverse
    }

    /// How well `name` matches, for ranking: the fuzzy score of a (not inverted) fuzzy filter,
    /// `None` for every other filter, which has no ranking.
    pub fn score(&self, name: &str) -> Option<i32> {
        match (&self.matcher, self.inverse) {
            (NameMatcher::Fuzzy(fuzzy), false) => fuzzy.score(name),
            _ => None,
        }
    }

    /// Whether this filter ranks its matches (a fuzzy filter that is not inverted).
    pub fn ranks(&self) -> bool {
        !self.inverse && matches!(self.matcher, NameMatcher::Fuzzy(_))
    }

    /// Whether every name this filter passes is also passed by `older`, so applying it to
    /// `older`'s result set gives the same rows as applying it to every object.
    ///
    /// True for a substring that grew (`fo` to `foo`), a fuzzy query that grew, and an inverse
    /// whose text shrank (`!foo` to `!fo` excludes more names). Anything else, a regular
    /// expression included, is recomputed from the cache.
    pub fn narrows(&self, older: &NameFilter) -> bool {
        if self.inverse != older.inverse {
            return false;
        }
        let (new, old) = if self.inverse {
            (&older.matcher, &self.matcher)
        } else {
            (&self.matcher, &older.matcher)
        };
        match (new, old) {
            (NameMatcher::Text(new), NameMatcher::Text(old)) => new.narrows(old),
            (NameMatcher::Fuzzy(new), NameMatcher::Fuzzy(old)) => new.narrows(old),
            _ => false,
        }
    }
}

/// ASCII-case-insensitive substring search without allocating; a non-ASCII name or needle falls
/// back to a lower-cased comparison.
fn contains_ignore_case(haystack: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    if haystack.is_ascii() && needle_lower.is_ascii() {
        let (h, n) = (haystack.as_bytes(), needle_lower.as_bytes());
        return h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n));
    }
    haystack.to_lowercase().contains(needle_lower)
}

/// The last line of a regex error (`error: unclosed group`), without the caret art.
fn one_line(error: &regex::Error) -> String {
    let text = error.to_string();
    text.lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("error: "))
        .map_or_else(
            || text.lines().next().unwrap_or("invalid pattern").to_owned(),
            str::to_owned,
        )
}
