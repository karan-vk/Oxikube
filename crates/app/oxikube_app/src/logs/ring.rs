//! [`LogBuffer`]: the bounded ring of lines a session keeps, with its line index.
//!
//! Every line gets a sequence number ([`LogEntry::seq`]) when it is appended: 0, 1, 2, ... in
//! stream order, never reused. The buffer keeps the newest `capacity` lines; appending past it
//! drops the oldest and counts them in [`dropped`](LogBuffer::dropped), which the viewer renders
//! as its "truncated" marker. The retained lines are always the contiguous seqs
//! `first_seq..next_seq`, so a line is found by index (0 is the oldest retained) or by seq in
//! O(1), and a range (what a virtualised list asks for) is a slice of the ring.

use std::collections::VecDeque;
use std::ops::Range;

use super::entry::LogEntry;

/// The retained lines of one session. See the module docs.
#[derive(Debug, Clone)]
pub struct LogBuffer {
    lines: VecDeque<LogEntry>,
    capacity: usize,
    /// Seq of `lines[0]` (and of the next line when empty).
    first_seq: u64,
    /// Lines dropped from the front over the buffer's life.
    dropped: u64,
    /// Lines the user cleared over the buffer's life (not counted in `dropped`).
    cleared: u64,
    /// What `dropped` was when the buffer was last cleared: lines dropped before it are gone on
    /// purpose, so they are no longer a truncation of what the buffer holds.
    dropped_at_clear: u64,
}

impl LogBuffer {
    /// An empty buffer that keeps at most `capacity` lines (at least one).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            // Never reserve the whole bound up front: a quiet pod would pay for a chatty one.
            lines: VecDeque::with_capacity(capacity.min(1_024)),
            capacity,
            first_seq: 0,
            dropped: 0,
            cleared: 0,
            dropped_at_clear: 0,
        }
    }

    /// Most lines kept.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Lines kept now.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Whether no line is kept.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Seq of the oldest retained line (the seq the next line gets when the buffer is empty).
    pub fn first_seq(&self) -> u64 {
        self.first_seq
    }

    /// Seq the next appended line gets; one past the newest retained line.
    pub fn next_seq(&self) -> u64 {
        self.first_seq + self.lines.len() as u64
    }

    /// Lines dropped from the front so far: the count behind the "truncated" marker.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Lines dropped from the front since the buffer was last cleared (all of them when it never
    /// was): what is missing before the first retained line, as the viewer's "truncated" marker
    /// counts it. Lines the user cleared are not part of it.
    pub fn dropped_since_clear(&self) -> u64 {
        self.dropped - self.dropped_at_clear
    }

    /// Lines the user cleared so far ([`clear`](Self::clear)); they were not dropped for space,
    /// so they are not part of [`dropped`](Self::dropped) and do not make the buffer truncated.
    pub fn cleared(&self) -> u64 {
        self.cleared
    }

    /// Whether older lines were dropped (show the "truncated" marker above the first line).
    pub fn is_truncated(&self) -> bool {
        self.dropped > 0
    }

    /// The `index`th retained line, 0 being the oldest.
    pub fn get(&self, index: usize) -> Option<&LogEntry> {
        self.lines.get(index)
    }

    /// The line with sequence number `seq`, if it is still retained.
    pub fn get_seq(&self, seq: u64) -> Option<&LogEntry> {
        self.get(self.index_of(seq)?)
    }

    /// The index of the line with sequence number `seq`, if it is still retained.
    pub fn index_of(&self, seq: u64) -> Option<usize> {
        let index = usize::try_from(seq.checked_sub(self.first_seq)?).ok()?;
        (index < self.lines.len()).then_some(index)
    }

    /// The newest line.
    pub fn last(&self) -> Option<&LogEntry> {
        self.lines.back()
    }

    /// The lines at `range` (indices, clamped to what is retained), oldest first.
    pub fn range(&self, range: Range<usize>) -> impl DoubleEndedIterator<Item = &LogEntry> + '_ {
        let end = range.end.min(self.lines.len());
        let start = range.start.min(end);
        self.lines.range(start..end)
    }

    /// The lines whose seq is in `range` (clamped to what is retained), oldest first.
    pub fn range_seq(&self, range: Range<u64>) -> impl DoubleEndedIterator<Item = &LogEntry> + '_ {
        let to_index =
            |seq: u64| usize::try_from(seq.saturating_sub(self.first_seq)).unwrap_or(usize::MAX);
        self.range(to_index(range.start)..to_index(range.end))
    }

    /// Every retained line, oldest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &LogEntry> + '_ {
        self.lines.iter()
    }

    /// Appends `entries`, numbering them, and drops the oldest lines past the capacity.
    /// Returns how many lines were dropped (including appended ones that did not fit).
    pub fn extend(&mut self, entries: impl IntoIterator<Item = LogEntry>) -> usize {
        let before = self.dropped;
        let next = self.next_seq();
        for (seq, mut entry) in (next..).zip(entries) {
            entry.seq = seq;
            self.lines.push_back(entry);
        }
        self.trim();
        usize::try_from(self.dropped - before).unwrap_or(usize::MAX)
    }

    /// Empties the buffer at the user's request. Seqs are never reused: the next line gets
    /// [`next_seq`](Self::next_seq) as before, and [`first_seq`](Self::first_seq) moves up to it.
    /// Returns how many lines were cleared.
    pub fn clear(&mut self) -> usize {
        let cleared = self.lines.len();
        self.lines.clear();
        self.lines.shrink_to(1_024);
        self.first_seq += cleared as u64;
        self.cleared += cleared as u64;
        self.dropped_at_clear = self.dropped;
        cleared
    }

    /// Changes the capacity (at least one), dropping the oldest lines when it shrinks. Returns how
    /// many were dropped.
    pub fn set_capacity(&mut self, capacity: usize) -> usize {
        let before = self.dropped;
        self.capacity = capacity.max(1);
        self.trim();
        usize::try_from(self.dropped - before).unwrap_or(usize::MAX)
    }

    fn trim(&mut self) {
        let excess = self.lines.len().saturating_sub(self.capacity);
        if excess == 0 {
            return;
        }
        self.lines.drain(..excess);
        self.first_seq += excess as u64;
        self.dropped += excess as u64;
        // A burst that grew the ring far past a quiet steady state gives the memory back.
        if self.lines.capacity() > 4 * self.capacity.max(1_024) {
            self.lines.shrink_to(self.capacity);
        }
    }
}
