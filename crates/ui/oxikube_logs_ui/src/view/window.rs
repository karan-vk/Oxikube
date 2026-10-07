//! [`LineWindow`]: the rows of a log view, kept in step with its session's [`LogDelta`]s. Plain
//! Rust, no gpui.
//!
//! The lines themselves stay in the session's ring buffer: the window only knows which seqs it
//! shows (`first_seq..next_seq`), so a frame reads the few rows on screen by seq and nothing is
//! copied per line. The rows are, top to bottom:
//!
//! 1. the "truncated" marker, while older lines were dropped to stay within `logs.buffer_lines`
//!    (the buffer's first seq is above the point the user last cleared at, 0 if never);
//! 2. one row per retained line, in stream order;
//! 3. the state row, while the session is not streaming (connecting, ended or failed).
//!
//! # Search
//!
//! A window may carry the [`MatchIndex`] of the view's search (E08-S03). In search mode it only
//! rides along (the view reads the count and the next match from it) and every line is a row. In
//! filter mode the window is *narrowed*: the lines are the index's matches, so the rows are the
//! matching lines only, and a delta changes the rows by what the index gained and lost.
//!
//! # Level filter
//!
//! With the level chips of the JSON mode (E08-S05) hiding some lines, the window keeps the seqs of
//! the lines that are rows, in order (8 bytes a line; `None`, and free, while nothing is hidden).
//! It composes with the search: while narrowed, the rows are the matches that also pass the chips.
//! A delta is told which of its candidate lines pass ([`LineWindow::apply_filtered`]: the new
//! matches while narrowed, the appended lines otherwise), and a changed filter or search replaces
//! the whole list ([`LineWindow::set_visible`], built from [`LineWindow::candidate_seqs`]).

use std::collections::VecDeque;
use std::ops::Range;

use oxikube_app::logs::{EndReason, IndexChange, LogBuffer, LogDelta, LogState, MatchIndex};

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

/// The rows of a log view. See the module docs.
#[derive(Debug, Clone)]
pub struct LineWindow {
    first_seq: u64,
    next_seq: u64,
    state: LogState,
    /// The seq the user cleared the view at: lines below it are gone on purpose, not dropped.
    cleared_to: u64,
    /// The search's matches, kept up to date with every delta.
    index: Option<MatchIndex>,
    /// Whether only the index's lines are rows (filter mode).
    narrowed: bool,
    /// Seqs of the lines that are rows while the level chips hide some lines, ascending; `None`
    /// when they hide none (the rows are then every line, or the index's while narrowed).
    visible: Option<VecDeque<u64>>,
}

impl Default for LineWindow {
    fn default() -> Self {
        Self {
            first_seq: 0,
            next_seq: 0,
            state: LogState::Connecting,
            cleared_to: 0,
            index: None,
            narrowed: false,
            visible: None,
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

    /// The carried index while the rows are its matches (filter mode).
    fn narrowing(&self) -> Option<&MatchIndex> {
        self.index.as_ref().filter(|_| self.narrowed)
    }

    /// Lines shown: the retained ones, the matching ones while narrowed, or those that pass the
    /// level chips while they hide some.
    pub fn line_count(&self) -> usize {
        match (&self.visible, self.narrowing()) {
            (Some(visible), _) => visible.len(),
            (None, Some(index)) => index.len(),
            (None, None) => self.retained_count(),
        }
    }

    /// The seq of the `line`th line shown.
    fn line_at(&self, line: usize) -> Option<u64> {
        match (&self.visible, self.narrowing()) {
            (Some(visible), _) => visible.get(line).copied(),
            (None, Some(index)) => index.get(line),
            (None, None) => (line < self.retained_count()).then(|| self.first_seq + line as u64),
        }
    }

    /// Whether the level chips or the hidden sources of a multi-pod view (E08-S04) hide some lines
    /// (the rows are the lines that pass them).
    pub fn is_level_filtered(&self) -> bool {
        self.visible.is_some()
    }

    /// Starts or stops the level filter: `Some(seqs)` makes exactly those lines (ascending, the
    /// rows without the filter that pass it, see [`candidate_seqs`](Self::candidate_seqs)) the
    /// rows, `None` shows every line (or every match) again. The rows change wholesale: the caller
    /// resets whatever it keeps per row.
    pub fn set_visible(&mut self, visible: Option<VecDeque<u64>>) {
        self.visible = visible;
    }

    /// The seqs that would be rows without the level filter: the index's matches while narrowed,
    /// every retained line otherwise. A new filter tests these against the buffer.
    pub fn candidate_seqs(&self) -> Box<dyn Iterator<Item = u64> + '_> {
        match self.narrowing() {
            Some(index) => Box::new((0..index.len()).filter_map(|i| index.get(i))),
            None => Box::new(self.first_seq..self.next_seq),
        }
    }

    /// The seqs in `first_seq..next_seq`.
    pub fn retained_seqs(&self) -> Range<u64> {
        self.first_seq..self.next_seq
    }

    /// Whether the "truncated" marker is shown: older lines were dropped to stay within the
    /// buffer (lines the user cleared do not count).
    pub fn is_truncated(&self) -> bool {
        self.first_seq > self.cleared_to
    }

    /// Whether the state row is shown. Not while streaming, and not for a stream that failed or
    /// closed unexpectedly: the recovery strip says that once, with the way out (an error is
    /// never drawn as a strip and again as a red row).
    pub fn shows_state(&self) -> bool {
        !matches!(
            self.state,
            LogState::Streaming | LogState::Failed(_) | LogState::Ended(EndReason::StreamClosed)
        )
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
        if let Some(seq) = self.line_at(line) {
            return Some(Row::Line(seq));
        }
        (line == self.line_count() && self.shows_state()).then_some(Row::State)
    }

    /// The row index of the line with `seq`, if it is shown.
    pub fn index_of(&self, seq: u64) -> Option<usize> {
        let line = match (&self.visible, self.narrowing()) {
            (Some(visible), _) => visible.binary_search(&seq).ok()?,
            (None, Some(index)) => index.position(seq)?,
            (None, None) => (self.first_seq..self.next_seq)
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
        let line = match (&self.visible, self.narrowing()) {
            (Some(visible), _) => visible.partition_point(|s| *s < seq),
            (None, Some(index)) => index.rank(seq),
            (None, None) => {
                usize::try_from(seq.saturating_sub(self.first_seq)).unwrap_or(usize::MAX)
            }
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

    /// The user cleared the view: every line goes (the session's buffer was emptied, `next_seq`
    /// is the seq its next line gets) and the lines before it are never "dropped". The level
    /// filter's rows go with them; the caller starts the search's index over (an empty one).
    /// Says how the rows moved; the state row stays.
    pub fn clear(&mut self, next_seq: u64) -> RowChange {
        let front_removed = self.marker_rows() + self.line_count();
        let dropped = self.line_count();
        self.first_seq = next_seq;
        self.next_seq = next_seq;
        self.cleared_to = next_seq;
        if let Some(visible) = self.visible.as_mut() {
            visible.clear();
        }
        RowChange {
            front_removed,
            dropped,
            ..RowChange::default()
        }
    }

    /// The matches of the search the window carries, if any.
    pub fn index(&self) -> Option<&MatchIndex> {
        self.index.as_ref()
    }

    /// Whether only the matching lines are rows (filter mode).
    pub fn is_narrowed(&self) -> bool {
        self.narrowing().is_some()
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
        self.apply_filtered(delta, buffer, |candidates| candidates)
    }

    /// The lines of `delta` the level filter has to test: the index's new matches while narrowed
    /// (`scanned` says how many), the appended lines still retained otherwise.
    fn delta_candidates(&self, delta: &LogDelta, scanned: Option<IndexChange>) -> Vec<u64> {
        match (self.narrowing(), scanned) {
            (Some(index), Some(change)) => (index.len().saturating_sub(change.appended)
                ..index.len())
                .filter_map(|i| index.get(i))
                .collect(),
            (Some(_), None) => Vec::new(),
            (None, _) => (delta.appended.start.max(delta.first_seq)..delta.appended.end).collect(),
        }
    }

    /// [`apply`](Self::apply) with the level filter on ([`set_visible`](Self::set_visible)):
    /// `admit` is given the delta's candidate lines (the index's new matches while narrowed, the
    /// appended lines still retained otherwise) and returns the ones that pass the chips. It is
    /// only called while the filter is on.
    pub fn apply_filtered(
        &mut self,
        delta: &LogDelta,
        buffer: Option<&LogBuffer>,
        admit: impl FnOnce(Vec<u64>) -> Vec<u64>,
    ) -> RowChange {
        let old_marker = self.marker_rows();
        let old_state = usize::from(self.shows_state());
        let held = self.line_count();
        let scanned = match (self.index.as_mut(), buffer) {
            (Some(index), Some(buffer)) => Some(index.catch_up(buffer)),
            _ => None,
        };
        let candidates = if self.visible.is_some() {
            self.delta_candidates(delta, scanned)
        } else {
            Vec::new()
        };
        let (dropped, appended) = if let Some(visible) = self.visible.as_mut() {
            let before = visible.len();
            while visible.front().is_some_and(|seq| *seq < delta.first_seq) {
                visible.pop_front();
            }
            let dropped = before - visible.len();
            let admitted = if candidates.is_empty() {
                Vec::new()
            } else {
                admit(candidates)
            };
            let appended = admitted.len();
            visible.extend(admitted);
            (dropped, appended)
        } else if self.is_narrowed() {
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
    fn clearing_removes_the_lines_without_a_marker_and_new_lines_carry_on() {
        let mut window = LineWindow::new();
        window.apply(&delta(0..5, 0, 0, LogState::Streaming), None);
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
        let change = window.apply(&delta(5..7, 5, 5, LogState::Streaming), None);
        assert_eq!(window.line_count(), 2);
        assert_eq!(window.row(0), Some(Row::Line(5)));
        assert_eq!(
            (change.front_removed, change.kept, change.tail_inserted),
            (0, 0, 2)
        );
        // Dropping from the ring after the clear is a truncation again, counted from the clear.
        window.apply(&delta(7..9, 1, 6, LogState::Streaming), None);
        assert_eq!(window.row(0), Some(Row::Truncated(1)));
    }

    #[test]
    fn clearing_empties_the_level_filters_rows_too() {
        let mut window = LineWindow::new();
        window.apply(&delta(0..6, 0, 0, LogState::Streaming), None);
        window.set_visible(Some([1, 3, 5].into_iter().collect()));
        assert_eq!(window.line_count(), 3);
        let change = window.clear(6);
        assert_eq!((window.line_count(), window.row_count()), (0, 0));
        assert_eq!((change.front_removed, change.dropped), (3, 3));
        window.apply_filtered(
            &delta(6..8, 0, 6, LogState::Streaming),
            None,
            |candidates| candidates.into_iter().filter(|s| s % 2 == 1).collect(),
        );
        assert_eq!(window.row(0), Some(Row::Line(7)));
    }

    #[test]
    fn clearing_keeps_the_state_row() {
        let mut window = LineWindow::new();
        window.apply(
            &delta(0..2, 0, 0, LogState::Ended(EndReason::PodFinished)),
            None,
        );
        assert_eq!(window.row_count(), 3);
        window.clear(2);
        assert_eq!(window.row_count(), 1);
        assert_eq!(window.row(0), Some(Row::State));
    }

    #[test]
    fn a_failure_or_an_unexplained_close_is_left_to_the_recovery_strip() {
        use oxikube_app::logs::LogFailure;
        use oxikube_domain::ErrorKind;
        let failed = LogState::Failed(LogFailure {
            kind: ErrorKind::Network,
            message: "connection refused".into(),
            retryable: true,
        });
        for state in [failed, LogState::Ended(EndReason::StreamClosed)] {
            let mut window = LineWindow::new();
            window.apply(&delta(0..2, 0, 0, state), None);
            assert_eq!(window.row_count(), 2, "the lines only, no state row");
            assert_eq!(window.row(2), None);
        }
    }

    #[test]
    fn an_ended_stream_gets_its_state_row_back() {
        let mut window = LineWindow::new();
        window.apply(&delta(0..2, 0, 0, LogState::Streaming), None);
        let change = window.apply(
            &delta(2..2, 0, 0, LogState::Ended(EndReason::PodFinished)),
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

    mod levels {
        use std::sync::Arc;

        use jiff::Timestamp;
        use oxikube_app::logs::{LogBuffer, LogEntry, LogFilter, MatchIndex};
        use oxikube_domain::log::LogLine;

        use super::*;

        /// Lines whose text starts with `e` match the search; seqs not divisible by 3 pass the
        /// level chips.
        fn pass(seq: u64) -> bool {
            !seq.is_multiple_of(3)
        }

        fn push(buffer: &mut LogBuffer, texts: &[String]) {
            buffer.extend(texts.iter().map(|text| {
                LogEntry::new(LogLine::new(Timestamp::UNIX_EPOCH, "p", "c", text.clone()))
            }));
        }

        fn rows(window: &LineWindow) -> Vec<u64> {
            (0..window.row_count())
                .filter_map(|i| match window.row(i) {
                    Some(Row::Line(seq)) => Some(seq),
                    _ => None,
                })
                .collect()
        }

        fn admit(candidates: Vec<u64>) -> Vec<u64> {
            candidates.into_iter().filter(|seq| pass(*seq)).collect()
        }

        /// Delta by delta, a level-filtered window shows exactly the lines of the model that pass.
        #[test]
        fn a_filtered_window_matches_a_naive_model_of_the_buffer() {
            let mut window = LineWindow::new();
            window.set_visible(Some(VecDeque::new()));
            let capacity = 10u64;
            let mut next = 0u64;
            for batch in [4u64, 1, 12, 3, 30, 2] {
                let seen_next = next;
                next += batch;
                let first = next.saturating_sub(capacity);
                let dropped =
                    (first.saturating_sub(window.first_seq())).min(window.retained_count() as u64);
                let change = window.apply_filtered(
                    &delta(
                        seen_next.max(first)..next,
                        dropped as usize,
                        first,
                        LogState::Streaming,
                    ),
                    None,
                    admit,
                );
                let want: Vec<u64> = (first..next).filter(|seq| pass(*seq)).collect();
                assert_eq!(rows(&window), want, "after {next} lines");
                assert_eq!(window.line_count(), want.len());
                assert_eq!(window.retained_count() as u64, next - first);
                assert_eq!(change.kept + change.appended, window.line_count());
                for (i, seq) in want.iter().enumerate() {
                    assert_eq!(window.index_of(*seq), Some(window.marker_rows() + i));
                }
                assert_eq!(window.index_of(first).is_some(), pass(first));
            }
        }

        #[test]
        fn row_near_seq_finds_the_neighbourhood_of_a_hidden_line() {
            let mut window = LineWindow::new();
            window.apply(&delta(0..10, 0, 0, LogState::Streaming), None);
            window.set_visible(Some([2, 5, 8].into_iter().collect()));
            assert_eq!(window.index_of(5), Some(1));
            assert_eq!(window.index_of(4), None);
            assert_eq!(window.row_near_seq(4), Some(1), "the next shown line");
            assert_eq!(window.row_near_seq(0), Some(0));
            assert_eq!(
                window.row_near_seq(9),
                Some(2),
                "past the end: the last line"
            );
            assert_eq!(
                window.seq_near(7),
                Some(8),
                "a row past the end maps to the last line"
            );
            window.set_visible(Some(VecDeque::new()));
            assert_eq!(window.row_near_seq(3), None);
            assert_eq!(window.seq_near(0), None);
            window.set_visible(None);
            assert_eq!(window.row_near_seq(4), Some(4));
        }

        /// While the search narrows the rows, the level chips narrow them further: the rows are
        /// the matches that pass, and a delta adds only the new matches that pass.
        #[test]
        fn the_chips_compose_with_a_narrowing_search() {
            let text = |seq: u64| {
                if seq.is_multiple_of(2) {
                    format!("e{seq}")
                } else {
                    format!("x{seq}")
                }
            };
            let mut buffer = LogBuffer::new(100);
            push(&mut buffer, &(0..12).map(text).collect::<Vec<_>>());
            let matcher = Arc::new(LogFilter::new("e").compile().unwrap());
            let mut index = MatchIndex::new(matcher);
            index.catch_up(&buffer);
            let mut window = LineWindow::new();
            window.apply(&delta(0..12, 0, 0, LogState::Streaming), None);
            window.set_index(Some(index), true);
            assert_eq!(rows(&window), [0, 2, 4, 6, 8, 10]);

            window.set_visible(Some(window.candidate_seqs().filter(|s| pass(*s)).collect()));
            assert_eq!(rows(&window), [2, 4, 8, 10], "matches that pass");
            assert_eq!(window.index_of(6), None);
            assert_eq!(window.index_of(3), None, "a line that does not match");

            // Lines 12..16 arrive: 12 and 14 match, only 14 passes.
            push(&mut buffer, &(12..16).map(text).collect::<Vec<_>>());
            window.apply_filtered(
                &delta(12..16, 0, 0, LogState::Streaming),
                Some(&buffer),
                admit,
            );
            assert_eq!(rows(&window), [2, 4, 8, 10, 14]);

            // Without the chips the matches are back; without the narrowing the passing lines.
            window.set_visible(None);
            assert_eq!(rows(&window), [0, 2, 4, 6, 8, 10, 12, 14]);
            window.set_narrowed(false);
            window.set_visible(Some(window.candidate_seqs().filter(|s| pass(*s)).collect()));
            assert_eq!(rows(&window), [1, 2, 4, 5, 7, 8, 10, 11, 13, 14]);
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
