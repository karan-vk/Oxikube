//! `n` / `N` over the matches of a text (E11-S06): one navigation rule for every view that
//! searches text, so the log view, the YAML and Describe tabs and the editor wrap the same way.
//!
//! | Module | Holds |
//! |---|---|
//! | `query` | [`FindQuery`]: a pattern compiled once (a plain substring, or a regex when it has regex syntax; case-insensitive); [`find_in_text`]: the byte ranges of its matches in a text |
//! | `navigator` | [`MatchList`], [`next_match`], [`previous_match`]: the rule; [`FindNavigator`]: the matches of one text and the current one |
//!
//! # The rule
//!
//! *Next* is the first match after the current one; with no current match, the first at or after
//! an anchor (the line at the top of the view); past the last match it wraps to the first.
//! *Previous* is the last match before the current one; with none current, the last match; before
//! the first it wraps to the last. A position is a `u64` the caller understands: a log line's
//! sequence number, a line number, or the byte where a match starts in a YAML text. A current
//! match that is gone (a log line the ring dropped) is skipped by position, not by index.
//!
//! The log view's [`MatchIndex`](crate::logs::MatchIndex) keeps its sequence numbers in a
//! list and calls [`next_match`] and [`previous_match`] on it; the YAML and Describe tabs of the
//! resource detail hold a [`FindNavigator`] over [`find_in_text`]'s ranges. The editor (E10) takes
//! the same helper when it gets a search bar.
//!
//! Plain Rust, no gpui.

mod navigator;
mod query;

#[cfg(test)]
mod tests;

pub use navigator::{FindNavigator, MatchList, next_match, previous_match};
pub use query::{FindMatches, FindQuery, MAX_MATCHES, find_in_text};
