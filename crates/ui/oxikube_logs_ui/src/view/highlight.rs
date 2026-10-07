//! How the search marks a visible row: the byte ranges of its matches and whether it is the
//! current match. Computed per row at draw time, for the rows on screen only.

use std::ops::Range;

use super::LogView;

/// Most highlight spans drawn in one row: a 16 KiB line of `e`s must not build thousands of runs.
pub(super) const MAX_SPANS: usize = 64;

/// How the search marks a row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Mark {
    /// Not marked.
    #[default]
    None,
    /// A line the inverse search matches: it has no span to paint, so the row is tinted.
    Matched,
    /// The match the user is on.
    Current,
}

/// What the search draws on one line.
#[derive(Clone, Debug, Default)]
pub(super) struct LineMarks {
    pub(super) mark: Mark,
    /// Byte ranges of the pattern's occurrences in the drawn text.
    pub(super) spans: Vec<Range<usize>>,
}

impl LogView {
    /// The byte ranges of row `index`'s drawn text (without its timestamp) that the search
    /// highlights.
    pub fn row_highlights(&self, index: usize) -> Vec<Range<usize>> {
        let Some(super::window::Row::Line(seq)) = self.window.row(index) else {
            return Vec::new();
        };
        let Some(text) = self.row_text(index) else {
            return Vec::new();
        };
        // `row_text` leads with the timestamp when it is shown: the spans are of the line's text.
        let text = match (self.options.timestamps, text.split_once(' ')) {
            (true, Some((_, line))) => line.to_owned(),
            _ => text,
        };
        self.line_marks(seq, &text).spans
    }

    /// The marks of the line `seq` drawn as `text` (nothing without a search).
    pub(super) fn line_marks(&self, seq: u64, text: &str) -> LineMarks {
        let Some(matcher) = self.search.state.matcher() else {
            return LineMarks::default();
        };
        let inverse = matcher.filter().inverse;
        let mark = if self.search.state.current() == Some(seq) {
            Mark::Current
        } else if inverse
            && !self.window.is_narrowed()
            && self.window.index().is_some_and(|index| index.contains(seq))
        {
            Mark::Matched
        } else {
            Mark::None
        };
        // An inverse search matches the lines *without* the pattern: nothing to paint in them.
        let spans = if inverse {
            Vec::new()
        } else {
            matcher.spans(text, MAX_SPANS)
        };
        LineMarks { mark, spans }
    }
}
