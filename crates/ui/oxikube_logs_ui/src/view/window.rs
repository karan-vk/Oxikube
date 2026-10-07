//! [`LineWindow`]: the rows of a log view, kept in step with its session's [`LogDelta`]s, and
//! [`Follow`], the autoscroll state with its "N new lines" count. Plain Rust, no gpui.
//!
//! The lines themselves stay in the session's ring buffer: the window only knows which seqs it
//! shows (`first_seq..next_seq`), so a frame reads the few rows on screen by seq and nothing is
//! copied per line. The rows are, top to bottom:
//!
//! 1. the "truncated" marker, while older lines were dropped to stay within `logs.buffer_lines`
//!    (the buffer's first seq is above the point the user last cleared at, 0 if never);
//! 2. one row per retained line, in stream order;
//! 3. the state row, while the session is not streaming (connecting, ended or failed).

use oxikube_app::logs::{LogDelta, LogState};

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineWindow {
    first_seq: u64,
    next_seq: u64,
    state: LogState,
    /// The seq the user cleared the view at: lines below it are gone on purpose, not dropped.
    cleared_to: u64,
}

impl Default for LineWindow {
    fn default() -> Self {
        Self {
            first_seq: 0,
            next_seq: 0,
            state: LogState::Connecting,
            cleared_to: 0,
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

    /// Lines shown.
    pub fn line_count(&self) -> usize {
        usize::try_from(self.next_seq - self.first_seq).unwrap_or(usize::MAX)
    }

    /// Whether the "truncated" marker is shown: older lines were dropped to stay within the
    /// buffer (lines the user cleared do not count).
    pub fn is_truncated(&self) -> bool {
        self.first_seq > self.cleared_to
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
            return Some(Row::Truncated(self.first_seq - self.cleared_to));
        }
        let line = index - marker;
        if line < self.line_count() {
            return Some(Row::Line(self.first_seq + line as u64));
        }
        (line == self.line_count() && self.shows_state()).then_some(Row::State)
    }

    /// The row index of the line with `seq`, if it is shown.
    pub fn index_of(&self, seq: u64) -> Option<usize> {
        (self.first_seq..self.next_seq)
            .contains(&seq)
            .then(|| self.marker_rows() + usize::try_from(seq - self.first_seq).unwrap_or(0))
    }

    /// The seq of the line at row `index`, or of the nearest line below it (the marker maps to
    /// the first line, the state row to the last). `None` when there is no line.
    pub fn seq_near(&self, index: usize) -> Option<u64> {
        if self.line_count() == 0 {
            return None;
        }
        let line = index.saturating_sub(self.marker_rows());
        Some((self.first_seq + line as u64).min(self.next_seq - 1))
    }

    /// The user cleared the view: every line goes (the session's buffer was emptied, `next_seq`
    /// is the seq its next line gets) and the lines before it are never "dropped". Says how the
    /// rows moved; the state row stays.
    pub fn clear(&mut self, next_seq: u64) -> RowChange {
        let front_removed = self.marker_rows() + self.line_count();
        let dropped = self.line_count();
        self.first_seq = next_seq;
        self.next_seq = next_seq;
        self.cleared_to = next_seq;
        RowChange {
            front_removed,
            dropped,
            ..RowChange::default()
        }
    }

    /// Applies `delta` and says how the rows moved.
    pub fn apply(&mut self, delta: &LogDelta) -> RowChange {
        let old_marker = self.marker_rows();
        let old_state = usize::from(self.shows_state());
        let held = self.line_count();
        let dropped = delta.dropped_front.min(held);
        let appended = usize::try_from(delta.appended.end - delta.appended.start).unwrap_or(0);

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

/// Autoscroll: on by default; scrolling up (or the autoscroll key) pauses it, and the view then
/// counts the lines that arrived since, by seq, for its "N new lines" pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Follow {
    on: bool,
    /// The window's `next_seq` when autoscroll paused.
    paused_at: u64,
}

impl Default for Follow {
    fn default() -> Self {
        Self {
            on: true,
            paused_at: 0,
        }
    }
}

impl Follow {
    /// Whether the view follows the newest line.
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Stops following; lines from `next_seq` on count as new.
    pub fn pause(&mut self, next_seq: u64) {
        if self.on {
            self.on = false;
            self.paused_at = next_seq;
        }
    }

    /// Follows again (the pill goes).
    pub fn resume(&mut self) {
        self.on = true;
    }

    /// Lines that arrived since autoscroll paused, by seq: dropping old lines from the ring
    /// buffer does not change it. `0` while following.
    pub fn new_lines(&self, next_seq: u64) -> u64 {
        if self.on {
            0
        } else {
            next_seq.saturating_sub(self.paused_at)
        }
    }

    /// The view was cleared and the next line has seq `next_seq`: lines that arrive from now on
    /// count for the pill, not the ones that were cleared.
    pub fn clear_to(&mut self, next_seq: u64) {
        if !self.on {
            self.paused_at = next_seq;
        }
    }

    /// Starts over for a new stream (its seqs start at 0 again), keeping on or off.
    pub fn restart(&mut self) {
        self.paused_at = 0;
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
        let change = window.apply(&delta(0..3, 0, 0, LogState::Streaming));
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
        window.apply(&delta(0..5, 0, 0, LogState::Streaming));
        let change = window.apply(&delta(5..7, 2, 2, LogState::Streaming));
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
    fn clearing_removes_the_lines_without_a_marker_and_new_lines_carry_on() {
        let mut window = LineWindow::new();
        window.apply(&delta(0..5, 0, 0, LogState::Streaming));
        let change = window.clear(5);
        assert_eq!(window.row_count(), 0);
        assert!(
            !window.is_truncated(),
            "cleared lines are not dropped lines"
        );
        assert_eq!(
            (change.front_removed, change.dropped, change.kept),
            (5, 5, 0)
        );
        // The session's next delta still describes the window the view had before the clear.
        let change = window.apply(&delta(5..7, 5, 5, LogState::Streaming));
        assert_eq!(window.line_count(), 2);
        assert_eq!(window.row(0), Some(Row::Line(5)));
        assert_eq!(
            (change.front_removed, change.kept, change.tail_inserted),
            (0, 0, 2)
        );
        // Dropping from the ring after the clear is a truncation again, counted from the clear.
        window.apply(&delta(7..9, 1, 6, LogState::Streaming));
        assert_eq!(window.row(0), Some(Row::Truncated(1)));
    }

    #[test]
    fn clearing_keeps_the_state_row() {
        let mut window = LineWindow::new();
        window.apply(&delta(0..2, 0, 0, LogState::Ended(EndReason::StreamClosed)));
        assert_eq!(window.row_count(), 3);
        window.clear(2);
        assert_eq!(window.row_count(), 1);
        assert_eq!(window.row(0), Some(Row::State));
    }

    #[test]
    fn an_ended_stream_gets_its_state_row_back() {
        let mut window = LineWindow::new();
        window.apply(&delta(0..2, 0, 0, LogState::Streaming));
        let change = window.apply(&delta(2..2, 0, 0, LogState::Ended(EndReason::StreamClosed)));
        assert_eq!(window.row(2), Some(Row::State));
        assert_eq!((change.tail_removed, change.tail_inserted), (0, 1));
        assert_eq!(
            window.seq_near(2),
            Some(1),
            "the state row maps to the last line"
        );
    }

    #[test]
    fn a_window_that_fell_behind_replaces_everything() {
        let mut window = LineWindow::new();
        window.apply(&delta(0..3, 0, 0, LogState::Streaming));
        // 10 more lines arrived and the buffer of 4 kept only seqs 9..13.
        let change = window.apply(&delta(9..13, 3, 9, LogState::Streaming));
        assert_eq!(window.line_count(), 4);
        assert_eq!(change.kept, 0);
        assert_eq!(change.front_removed, 3);
        assert_eq!(window.row(1), Some(Row::Line(9)));
    }

    #[test]
    fn the_pill_counts_by_seq_not_by_rows() {
        let mut follow = Follow::default();
        assert_eq!(follow.new_lines(100), 0);
        follow.pause(100);
        follow.pause(150); // already paused: the first point stays
        assert_eq!(follow.new_lines(130), 30);
        // The ring buffer dropped 1 000 old lines meanwhile: the count does not care.
        assert_eq!(follow.new_lines(1_300), 1_200);
        follow.clear_to(2_000);
        assert_eq!(follow.new_lines(2_005), 5, "a clear starts the count over");
        follow.resume();
        assert!(follow.is_on());
        assert_eq!(follow.new_lines(2_000), 0);
    }
}
