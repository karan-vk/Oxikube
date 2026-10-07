//! Search and filter over a session's lines (E08-S03).
//!
//! The viewer's `/` bar, the next / previous match and the agent's `grep` argument
//! (`get_logs`, E08-S09) all ask one question of a line: does it match? The answer lives here,
//! independent of any view.
//!
//! | Piece | Where |
//! |---|---|
//! | the user's pattern: regex, case toggle, inverse | [`LogFilter`] (`pattern`) |
//! | the pattern compiled once per edit; `matches(&str)` and the highlight spans | [`LogMatcher`], [`FilterError`] (`pattern`) |
//! | the sorted seqs of the matching lines, kept up to date incrementally over the ring | [`MatchIndex`], [`IndexChange`] (`index`) |
//!
//! # Incremental over the ring buffer
//!
//! A [`MatchIndex`] remembers how far it has tested the buffer (`scanned_to`). [`MatchIndex::scan`]
//! tests only the lines appended since, and drops the matches of lines the ring dropped, so a
//! streaming session costs one regex test per new line however large the buffer is. Building an
//! index over a full buffer (a pattern edit) is the same call in bounded chunks: the viewer runs
//! those on a background executor and publishes the finished index, so the render never waits.
//! The result always equals a naive scan of the retained lines (a property test pins it).
//!
//! # Stored text only
//!
//! Matching reads [`LogEntry::text`](super::LogEntry): nothing is shaped or allocated per line.
//! Highlight spans are computed by the view for the rows it draws.

mod index;
mod pattern;

pub use index::{IndexChange, MatchIndex};
pub use pattern::{FilterError, LogFilter, LogMatcher};
