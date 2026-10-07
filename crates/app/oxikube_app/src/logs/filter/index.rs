//! [`MatchIndex`]: the sorted seqs of the lines a [`LogMatcher`] matches, kept up to date
//! incrementally over a session's ring buffer.

use std::collections::VecDeque;
use std::sync::Arc;

use super::pattern::LogMatcher;
use crate::logs::LogBuffer;

/// How a [`MatchIndex::scan`] changed the index: the viewer keeps the rows of a filtered list in
/// step with it (remove `dropped_front` rows at the top, add `appended` at the bottom).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexChange {
    /// Matches removed from the front: their lines were dropped from the ring.
    pub dropped_front: usize,
    /// Matches added at the back.
    pub appended: usize,
    /// Lines tested by this call.
    pub tested: u64,
}

/// The matching lines of one session, by seq, oldest first. See the module docs.
#[derive(Clone, Debug)]
pub struct MatchIndex {
    matcher: Arc<LogMatcher>,
    matches: VecDeque<u64>,
    /// The seq of the next line to test: everything below it was tested (or dropped).
    scanned_to: u64,
}

impl MatchIndex {
    /// An empty index for `matcher`, to be filled by [`scan`](Self::scan).
    pub fn new(matcher: Arc<LogMatcher>) -> Self {
        Self {
            matcher,
            matches: VecDeque::new(),
            scanned_to: 0,
        }
    }

    /// Matching lines retained.
    pub fn len(&self) -> usize {
        self.matches.len()
    }

    /// Whether no retained line matches.
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// The `index`th match (0 is the oldest).
    pub fn get(&self, index: usize) -> Option<u64> {
        self.matches.get(index).copied()
    }

    /// The seqs of the matches, oldest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = u64> + '_ {
        self.matches.iter().copied()
    }

    /// The position of `seq` among the matches, if it is one.
    pub fn position(&self, seq: u64) -> Option<usize> {
        let at = self.matches.partition_point(|&m| m < seq);
        (self.matches.get(at) == Some(&seq)).then_some(at)
    }

    /// Whether the line `seq` is a retained match.
    pub fn contains(&self, seq: u64) -> bool {
        self.position(seq).is_some()
    }

    /// How many matches are below `seq` (a line's row among the matches, matched or not).
    pub fn rank(&self, seq: u64) -> usize {
        self.matches.partition_point(|&m| m < seq)
    }

    /// The lines of `buffer` this index has not tested yet.
    pub fn pending(&self, buffer: &LogBuffer) -> u64 {
        buffer
            .next_seq()
            .saturating_sub(self.scanned_to.max(buffer.first_seq()))
    }

    /// Whether every line of `buffer` was tested.
    pub fn is_caught_up(&self, buffer: &LogBuffer) -> bool {
        self.pending(buffer) == 0
    }

    /// Brings the index up to `buffer`: drops the matches of lines the ring dropped, then tests
    /// the lines appended since the last call, at most `max_lines` of them (call again while
    /// [`is_caught_up`](Self::is_caught_up) is false). Lines that were appended and dropped
    /// again before any call are never tested.
    ///
    /// A buffer that restarted (its newest seq is below what was tested: a new session) empties
    /// the index first.
    pub fn scan(&mut self, buffer: &LogBuffer, max_lines: usize) -> IndexChange {
        let mut change = IndexChange::default();
        if self.scanned_to > buffer.next_seq() {
            change.dropped_front = self.matches.len();
            self.matches.clear();
            self.scanned_to = 0;
        }
        let first = buffer.first_seq();
        let stale = self.matches.partition_point(|&m| m < first);
        self.matches.drain(..stale);
        change.dropped_front += stale;

        let start = self.scanned_to.max(first);
        let end = buffer
            .next_seq()
            .min(start.saturating_add(max_lines as u64));
        let before = self.matches.len();
        for entry in buffer.range_seq(start..end) {
            if self.matcher.matches(&entry.text) {
                self.matches.push_back(entry.seq);
            }
        }
        change.appended = self.matches.len() - before;
        change.tested = end.saturating_sub(start);
        self.scanned_to = end.max(self.scanned_to);
        change
    }

    /// [`scan`](Self::scan) over everything pending.
    pub fn catch_up(&mut self, buffer: &LogBuffer) -> IndexChange {
        self.scan(buffer, usize::MAX)
    }

    /// The oldest match.
    pub fn first(&self) -> Option<u64> {
        self.matches.front().copied()
    }

    /// The newest match.
    pub fn last(&self) -> Option<u64> {
        self.matches.back().copied()
    }

    /// The first match at or after `seq`.
    pub fn at_or_after(&self, seq: u64) -> Option<u64> {
        self.get(self.matches.partition_point(|&m| m < seq))
    }

    /// The first match after `seq`.
    pub fn after(&self, seq: u64) -> Option<u64> {
        self.get(self.matches.partition_point(|&m| m <= seq))
    }

    /// The last match before `seq`.
    pub fn before(&self, seq: u64) -> Option<u64> {
        let at = self.matches.partition_point(|&m| m < seq);
        at.checked_sub(1).and_then(|at| self.get(at))
    }

    /// The match to go to for "next": the first one after `current` (which may have been dropped
    /// from the ring meanwhile: the next retained one is found by seq), else the first at or
    /// after `anchor` (the line at the top of the screen), wrapping to the first match at the
    /// end. `None` when nothing matches.
    pub fn next(&self, current: Option<u64>, anchor: u64) -> Option<u64> {
        let found = match current {
            Some(current) => self.after(current),
            None => self.at_or_after(anchor),
        };
        found.or_else(|| self.first())
    }

    /// The match to go to for "previous": the last one before `current` (else the newest match),
    /// wrapping to the newest at the start. `None` when nothing matches.
    pub fn prev(&self, current: Option<u64>) -> Option<u64> {
        current
            .and_then(|current| self.before(current))
            .or_else(|| self.last())
    }
}
