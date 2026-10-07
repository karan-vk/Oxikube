//! [`LineWindow`]: the rows of a log view, kept in step with its session's [`LogDelta`]s. Plain
//! Rust, no gpui.
//!
//! The lines themselves stay in the session's ring buffer: the window only knows which seqs it
//! shows (`first_seq..next_seq`), so a frame reads the few rows on screen by seq and nothing is
//! copied per line. The rows are, top to bottom:
//!
//! 1. the "truncated" marker, while older lines were dropped to stay within `logs.buffer_lines`
//!    (the buffer's first seq is above 0);
//! 2. one row per retained line, in stream order;
//! 3. the state row, while the session is not streaming (connecting, ended or failed).
//!
//! # Search
//!
//! A window may carry the [`MatchIndex`] of the view's search (E08-S03). In search mode it only
//! rides along (the view reads the count and the next match from it) and every line is a row. In
//! filter mode the window is *narrowed*: the lines are the index's matches, so the rows are the
//! matching lines only, and a delta changes the rows by what the index gained and lost.

use oxikube_app::logs::{LogBuffer, LogDelta, LogState, MatchIndex};

/// One row of a log view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// The "truncated" marker: this many older lines were dropped.
    Truncated(u64),
    /// The line with this seq.
    Line(u64),
    /// The session's state (connecting, ended, failed).
    State,
}

/// How a delta changed the rows, for a renderer that keeps per-row state (the wrapped list):
/// first replace `front_removed` rows at the top with `front_inserted`, then replace
/// `tail_removed` rows after the lines that stayed with `tail_inserted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RowChange {
    /// Rows removed from the top: the old marker and the dropped lines.
    pub front_removed: usize,
    /// Rows inserted at the top: the new marker.
    pub front_inserted: usize,
    /// Lines that stayed (between the front and the tail edits).
    pub kept: usize,
    /// Rows removed after the kept lines: the old state row.
    pub tail_removed: usize,
    /// Rows inserted after the kept lines: the appended lines and the new state row.
    pub tail_inserted: usize,
    /// Lines appended.
    pub appended: usize,
    /// Lines dropped from the front.
    pub dropped: usize,
}

impl RowChange {
    /// Whether nothing moved.
    pub fn is_empty(&self) -> bool {
        self.front_removed == 0
            && self.front_inserted == 0
            && self.tail_removed == 0
            && self.tail_inserted == 0
    }
}

/// The rows of a log view. See the [module docs](self).
#[derive(Debug, Clone)]
pub struct LineWindow {
    first_seq: u64,
    next_seq: u64,
    state: LogState,
    /// The search's matches, kept up to date with every delta.
    index: Option<MatchIndex>,
    /// Whether only the index's lines are rows (filter mode).
    narrowed: bool,
}

impl Default for LineWindow {
    fn default() -> Self {
        Self {
            first_seq: 0,
            next_seq: 0,
            state: LogState::Connecting,
            index: None,
            narrowed: false,
        }
    }
}

impl LineWindow {
    /// A window for a session that has not delivered anything yet (connecting).
    pub fn new() -> Self {
        Self::default()
    }

    /// A window that shows no stream at all, only the state row with `state` (a view whose
    /// cluster is not connected).
    pub fn with_state(state: LogState) -> Self {
        Self {
            state,
            ..Self::default()
        }
    }

    /// Seq of the oldest line shown.
    pub fn first_seq(&self) -> u64 {
        self.first_seq
    }

    /// One past the newest line's seq: every line the session ever read is below it.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// The session's state as of the last delta.
    pub fn state(&self) -> &LogState {
        &self.state
    }

    /// Lines the session retained, as far as the last delta says.
    pub fn retained_count(&self) -> usize {
        usize::try_from(self.next_seq - self.first_seq).unwrap_or(usize::MAX)
    }

    /// Lines shown: the retained ones, or the matching ones while narrowed.
    pub fn line_count(&self) -> usize {
        match &self.index {
            Some(index) if self.narrowed => index.len(),
            _ => self.retained_count(),
        }
    }

    /// The seq of the `line`th line shown.
    fn line_at(&self, line: usize) -> Option<u64> {
        match &self.index {
            Some(index) if self.narrowed => index.get(line),
            _ => (line < self.retained_count()).then(|| self.first_seq + line as u64),
        }
    }

    /// Whether the "truncated" marker is shown (older lines were dropped).
    pub fn is_truncated(&self) -> bool {
        self.first_seq > 0
    }

    /// Whether the state row is shown.
    pub fn shows_state(&self) -> bool {
        self.state != LogState::Streaming
    }

    fn marker_rows(&self) -> usize {
        usize::from(self.is_truncated())
    }

    /// Every row: marker, lines, state.
    pub fn row_count(&self) -> usize {
        self.marker_rows() + self.line_count() + usize::from(self.shows_state())
    }

    /// The row at `index`; `None` past the end.
    pub fn row(&self, index: usize) -> Option<Row> {
        let marker = self.marker_rows();
        if index < marker {
            return Some(Row::Truncated(self.first_seq));
        }
        let line = index - marker;
        if let Some(seq) = self.line_at(line) {
            return Some(Row::Line(seq));
        }
        (line == self.line_count() && self.shows_state()).then_some(Row::State)
    }

    /// The row index of the line with `seq`, if it is shown.
    pub fn index_of(&self, seq: u64) -> Option<usize> {
        let line = match &self.index {
            Some(index) if self.narrowed => index.position(seq)?,
            _ => (self.first_seq..self.next_seq)
                .contains(&seq)
                .then(|| usize::try_from(seq - self.first_seq).unwrap_or(0))?,
        };
        Some(self.marker_rows() + line)
    }

    /// The row of the line with `seq`, or of the nearest line shown after it (the last line when
    /// there is none): where to scroll to keep a line in sight when the rows were rebuilt.
    pub fn row_near_seq(&self, seq: u64) -> Option<usize> {
        let count = self.line_count();
        if count == 0 {
            return None;
        }
        let line = match &self.index {
            Some(index) if self.narrowed => index.rank(seq),
            _ => usize::try_from(seq.saturating_sub(self.first_seq)).unwrap_or(usize::MAX),
        };
        Some(self.marker_rows() + line.min(count - 1))
    }

    /// The seq of the line at row `index`, or of the nearest line below it (the marker maps to
    /// the first line, the state row to the last). `None` when there is no line.
    pub fn seq_near(&self, index: usize) -> Option<u64> {
        let count = self.line_count();
        if count == 0 {
            return None;
        }
        self.line_at(index.saturating_sub(self.marker_rows()).min(count - 1))
    }

    /// The matches of the search the window carries, if any.
    pub fn index(&self) -> Option<&MatchIndex> {
        self.index.as_ref()
    }

    /// Whether only the matching lines are rows (filter mode).
    pub fn is_narrowed(&self) -> bool {
        self.narrowed && self.index.is_some()
    }

    /// Carries `index` (the search's matches) from now on; `narrowed` makes its lines the rows.
    /// The rows change wholesale: the caller resets whatever it keeps per row.
    pub fn set_index(&mut self, index: Option<MatchIndex>, narrowed: bool) {
        self.narrowed = narrowed && index.is_some();
        self.index = index;
    }

    /// Narrows the rows to the index's lines (filter mode), or shows every line again. The rows
    /// change wholesale: the caller resets whatever it keeps per row.
    pub fn set_narrowed(&mut self, narrowed: bool) {
        self.narrowed = narrowed && self.index.is_some();
    }

    /// Applies `delta` and says how the rows moved. `buffer` (the session's, read for this
    /// delta) brings the carried index up to date first; while narrowed the rows follow what the
    /// index gained and lost, otherwise every line is a row.
    pub fn apply(&mut self, delta: &LogDelta, buffer: Option<&LogBuffer>) -> RowChange {
        let old_marker = self.marker_rows();
        let old_state = usize::from(self.shows_state());
        let held = self.line_count();
        let scanned = match (self.index.as_mut(), buffer) {
            (Some(index), Some(buffer)) => Some(index.catch_up(buffer)),
            _ => None,
        };
        let (dropped, appended) = if self.is_narrowed() {
            scanned.map_or((0, 0), |change| {
                (change.dropped_front.min(held), change.appended)
            })
        } else {
            let appended = usize::try_from(delta.appended.end - delta.appended.start).unwrap_or(0);
            (delta.dropped_front.min(held), appended)
        };

        self.first_seq = delta.first_seq;
        self.next_seq = delta.appended.end.max(delta.first_seq);
        self.state = delta.state.clone();

        RowChange {
            front_removed: old_marker + dropped,
            front_inserted: self.marker_rows(),
            kept: held - dropped,
            tail_removed: old_state,
            tail_inserted: appended + usize::from(self.shows_state()),
            appended,
            dropped,
        }
    }
}

#[cfg(test)]
mod tests {
    use oxikube_app::logs::EndReason;

    use super::*;

    fn delta(
        appended: std::ops::Range<u64>,
        dropped: usize,
        first: u64,
        state: LogState,
    ) -> LogDelta {
        LogDelta {
            appended,
            dropped_front: dropped,
            first_seq: first,
            state,
        }
    }

    #[test]
    fn a_connecting_window_is_one_state_row() {
        let window = LineWindow::new();
        assert_eq!(window.row_count(), 1);
        assert_eq!(window.row(0), Some(Row::State));
        assert_eq!(window.row(1), None);
        assert_eq!(window.seq_near(0), None);
    }

    #[test]
    fn appended_lines_become_rows_and_the_state_row_goes_while_streaming() {
        let mut window = LineWindow::new();
        let change = window.apply(&delta(0..3, 0, 0, LogState::Streaming), None);
        assert_eq!(window.row_count(), 3);
        assert_eq!(window.row(2), Some(Row::Line(2)));
        assert_eq!(
            change,
            RowChange {
                front_removed: 0,
                front_inserted: 0,
                kept: 0,
                tail_removed: 1,
                tail_inserted: 3,
                appended: 3,
                dropped: 0,
            }
        );
    }

    #[test]
    fn dropping_old_lines_shows_the_marker_and_keeps_seqs() {
        let mut window = LineWindow::new();
        window.apply(&delta(0..5, 0, 0, LogState::Streaming), None);
        let change = window.apply(&delta(5..7, 2, 2, LogState::Streaming), None);
        assert!(window.is_truncated());
        assert_eq!(window.row(0), Some(Row::Truncated(2)));
        assert_eq!(window.row(1), Some(Row::Line(2)));
        assert_eq!(window.row(5), Some(Row::Line(6)));
        assert_eq!(window.row_count(), 6);
        assert_eq!(window.index_of(6), Some(5));
        assert_eq!(window.index_of(1), None);
        assert_eq!((change.front_removed, change.front_inserted), (2, 1));
        assert_eq!((change.kept, change.tail_inserted), (3, 2));
    }

    #[test]
    fn an_ended_stream_gets_its_state_row_back() {
        let mut window = LineWindow::new();
        window.apply(&delta(0..2, 0, 0, LogState::Streaming), None);
        let change = window.apply(
            &delta(2..2, 0, 0, LogState::Ended(EndReason::StreamClosed)),
            None,
        );
        assert_eq!(window.row(2), Some(Row::State));
        assert_eq!((change.tail_removed, change.tail_inserted), (0, 1));
        assert_eq!(
            window.seq_near(2),
            Some(1),
            "the state row maps to the last line"
        );
    }

    mod narrowed {
        use std::sync::Arc;

        use jiff::Timestamp;
        use oxikube_app::logs::{LogBuffer, LogEntry, LogFilter, MatchIndex};
        use oxikube_domain::log::LogLine;

        use super::*;

        fn buffer(capacity: usize) -> LogBuffer {
            LogBuffer::new(capacity)
        }

        fn push(buffer: &mut LogBuffer, texts: &[&str]) {
            buffer.extend(texts.iter().map(|text| {
                LogEntry::new(LogLine::new(
                    Timestamp::UNIX_EPOCH,
                    "p",
                    "c",
                    (*text).to_owned(),
                ))
            }));
        }

        fn narrowed_window(buffer: &LogBuffer) -> LineWindow {
            let matcher = Arc::new(LogFilter::new("e").compile().unwrap());
            let mut index = MatchIndex::new(matcher);
            index.catch_up(buffer);
            let mut window = LineWindow::new();
            window.apply(
                &delta(
                    buffer.first_seq()..buffer.next_seq(),
                    0,
                    buffer.first_seq(),
                    LogState::Streaming,
                ),
                None,
            );
            window.set_index(Some(index), true);
            window
        }

        #[test]
        fn only_the_matches_are_rows_and_seqs_map_both_ways() {
            let mut buffer = buffer(100);
            push(&mut buffer, &["e0", "x1", "e2", "x3", "x4", "e5"]);
            let window = narrowed_window(&buffer);
            assert!(window.is_narrowed());
            assert_eq!((window.line_count(), window.retained_count()), (3, 6));
            assert_eq!(window.row(0), Some(Row::Line(0)));
            assert_eq!(window.row(1), Some(Row::Line(2)));
            assert_eq!(window.row(2), Some(Row::Line(5)));
            assert_eq!(window.row(3), None);
            assert_eq!(window.index_of(2), Some(1));
            assert_eq!(window.index_of(3), None, "a hidden line is no row");
            // Where to scroll for a hidden line: the next row shown after it.
            assert_eq!(window.row_near_seq(3), Some(2));
            assert_eq!(window.row_near_seq(99), Some(2));
            assert_eq!(window.seq_near(1), Some(2));
            assert_eq!(window.seq_near(50), Some(5));
        }

        #[test]
        fn a_delta_changes_the_rows_by_the_matches_gained_and_lost() {
            let mut buffer = buffer(4);
            push(&mut buffer, &["e0", "x1", "e2", "x3"]);
            let mut window = narrowed_window(&buffer);
            assert_eq!(window.row_count(), 2);

            // Two lines arrive (one match) and the ring drops seqs 0 and 1 (one match).
            push(&mut buffer, &["e4", "x5"]);
            let change = window.apply(&delta(4..6, 2, 2, LogState::Streaming), Some(&buffer));
            assert_eq!(window.line_count(), 2);
            assert_eq!(window.row(0), Some(Row::Truncated(2)));
            assert_eq!(window.row(1), Some(Row::Line(2)));
            assert_eq!(window.row(2), Some(Row::Line(4)));
            assert_eq!(change.dropped, 1);
            assert_eq!(change.appended, 1);
            assert_eq!((change.front_removed, change.front_inserted), (1, 1));
            assert_eq!((change.kept, change.tail_inserted), (1, 1));
        }

        #[test]
        fn without_narrowing_the_index_rides_along_and_every_line_is_a_row() {
            let mut buffer = buffer(100);
            push(&mut buffer, &["e0", "x1", "e2"]);
            let mut window = narrowed_window(&buffer);
            window.set_narrowed(false);
            assert!(!window.is_narrowed());
            assert_eq!(window.line_count(), 3);
            assert_eq!(window.index().unwrap().len(), 2);
            push(&mut buffer, &["e3"]);
            let change = window.apply(&delta(3..4, 0, 0, LogState::Streaming), Some(&buffer));
            assert_eq!(change.appended, 1);
            assert_eq!(
                window.index().unwrap().len(),
                3,
                "the index followed the delta"
            );
        }
    }

    #[test]
    fn a_window_that_fell_behind_replaces_everything() {
        let mut window = LineWindow::new();
        window.apply(&delta(0..3, 0, 0, LogState::Streaming), None);
        // 10 more lines arrived and the buffer of 4 kept only seqs 9..13.
        let change = window.apply(&delta(9..13, 3, 9, LogState::Streaming), None);
        assert_eq!(window.line_count(), 4);
        assert_eq!(change.kept, 0);
        assert_eq!(change.front_removed, 3);
        assert_eq!(window.row(1), Some(Row::Line(9)));
    }
}
