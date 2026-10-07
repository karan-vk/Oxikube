//! [`Merger`]: the reorder window that merges per-stream batches by server timestamp.
//!
//! Streams arrive with different latencies, so a line is not committed to the merged buffer the
//! moment it is read: it waits in a min-heap, ordered by `(server timestamp, stream id, per-stream
//! sequence)`, for the *reorder window* after it arrived, and lines of slower streams that arrive
//! meanwhile slot in before it. The key is the whole ordering rule:
//!
//! * **Server timestamp first**: the merge is by what the kubelet stamped, not by when the client
//!   read the line.
//! * **Stable tiebreak**: equal timestamps (the kubelet stamps to the nanosecond, but a burst can
//!   still collide) are ordered by stream id, then by the line's sequence in its own stream, so the
//!   same input always merges to the same output.
//! * **Never reorder within one stream**: a line's key timestamp is lifted to the previous line's
//!   when its own stamp is older (clock steps, or a container that restarted), so lines of one pod
//!   keep the order the server sent them whatever the clock skew between pods.
//!
//! Lines older than the window when they arrive (the backlog of a pod that joined late) are
//! committed at the next flush, in best-effort order: they cannot be placed before lines already
//! committed. A line waits at most one window past the arrival of the line before it in merge
//! order, and no more than `max_pending` lines wait at all.
//!
//! Plain data and a caller-supplied clock: no tasks, no timers; the driver decides when to
//! [`flush`](Merger::flush).

use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BinaryHeap};
use std::time::Duration;

use jiff::Timestamp;

use super::sources::SourceId;
use crate::logs::entry::LogEntry;

/// A line waiting in the window.
struct Pending {
    /// `(key timestamp, stream id, sequence in the stream)`: the merge order.
    key: (Timestamp, SourceId, u64),
    /// When the line was read (the local clock).
    arrived: Timestamp,
    entry: LogEntry,
}

impl PartialEq for Pending {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for Pending {}

impl PartialOrd for Pending {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Pending {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key.cmp(&other.key)
    }
}

/// Where one stream is: its newest key timestamp and the sequence of its next line.
#[derive(Default)]
struct StreamClock {
    newest: Option<Timestamp>,
    next_seq: u64,
}

/// The reorder window. See the [module docs](self).
pub(super) struct Merger {
    window: Duration,
    heap: BinaryHeap<Reverse<Pending>>,
    streams: BTreeMap<SourceId, StreamClock>,
}

impl Merger {
    /// A merger that holds lines for `window`.
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            heap: BinaryHeap::new(),
            streams: BTreeMap::new(),
        }
    }

    /// Takes a batch read from `source` at `now`, in the order the stream sent it.
    pub fn push(&mut self, source: SourceId, now: Timestamp, entries: Vec<LogEntry>) {
        let clock = self.streams.entry(source).or_default();
        for entry in entries {
            let ts = clock.newest.map_or(entry.ts, |newest| entry.ts.max(newest));
            clock.newest = Some(ts);
            let seq = clock.next_seq;
            clock.next_seq += 1;
            self.heap.push(Reverse(Pending {
                key: (ts, source, seq),
                arrived: now,
                entry,
            }));
        }
    }

    /// Whether lines are waiting.
    pub fn has_pending(&self) -> bool {
        !self.heap.is_empty()
    }

    /// Lines waiting.
    #[cfg(test)]
    pub fn pending(&self) -> usize {
        self.heap.len()
    }

    /// The lines that are due at `now`, in merge order: those that waited a full window, and the
    /// oldest ones beyond `max_pending` (which never wait).
    pub fn flush(&mut self, now: Timestamp, max_pending: usize) -> Vec<LogEntry> {
        let mut out = Vec::new();
        while let Some(Reverse(top)) = self.heap.peek() {
            let due = top
                .arrived
                .checked_add(self.window)
                .map_or(true, |due| due <= now);
            if !due && self.heap.len() <= max_pending {
                break;
            }
            out.extend(self.pop());
        }
        out
    }

    /// The oldest line in merge order.
    fn pop(&mut self) -> Option<LogEntry> {
        self.heap.pop().map(|Reverse(line)| line.entry)
    }

    /// The oldest lines beyond `max_pending`, in merge order: what must go out even while the
    /// window is held (the start-up barrier), so the waiting lines stay bounded by the buffer
    /// they would be committed to.
    pub fn overflow(&mut self, max_pending: usize) -> Vec<LogEntry> {
        let excess = self.heap.len().saturating_sub(max_pending);
        (0..excess).map_while(|_| self.pop()).collect()
    }

    /// Every waiting line, in merge order, whatever the window says (the streams ended).
    pub fn drain(&mut self) -> Vec<LogEntry> {
        std::iter::from_fn(|| self.pop()).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    const WINDOW: Duration = Duration::from_millis(300);

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_millisecond(1_760_000_000_000 + ms).unwrap()
    }

    fn line(pod: &str, ts_ms: i64, text: &str) -> LogEntry {
        LogEntry {
            seq: 0,
            ts: at(ts_ms),
            pod: Arc::from(pod),
            container: Arc::from("app"),
            text: Arc::from(text),
            truncated: false,
            level: None,
        }
    }

    fn texts(lines: &[LogEntry]) -> Vec<&str> {
        lines.iter().map(|l| &*l.text).collect()
    }

    #[test]
    fn lines_wait_for_the_window_and_come_out_by_server_timestamp() {
        let mut merger = Merger::new(WINDOW);
        // Pod a's lines arrive first, pod b's (slower connection) 100 ms later, but b's stamps
        // fall between a's.
        merger.push(1, at(1_000), vec![line("a", 10, "a1"), line("a", 30, "a2")]);
        merger.push(2, at(1_100), vec![line("b", 20, "b1"), line("b", 40, "b2")]);
        assert!(
            merger.flush(at(1_299), 100).is_empty(),
            "a's lines are not due yet"
        );
        let due = merger.flush(at(1_300), 100);
        assert_eq!(
            texts(&due),
            ["a1"],
            "a1 is due, a2 and b's lines wait their own window"
        );
        let due = merger.flush(at(1_400), 100);
        assert_eq!(texts(&due), ["b1", "a2", "b2"], "merged by timestamp");
    }

    #[test]
    fn a_late_stream_slots_in_before_lines_that_are_still_waiting() {
        let mut merger = Merger::new(WINDOW);
        merger.push(1, at(0), vec![line("a", 100, "a-late-stamp")]);
        merger.push(2, at(200), vec![line("b", 50, "b-early-stamp")]);
        let all = merger.flush(at(10_000), 100);
        assert_eq!(texts(&all), ["b-early-stamp", "a-late-stamp"]);
    }

    #[test]
    fn equal_timestamps_order_by_stream_then_by_position_in_the_stream() {
        let mut merger = Merger::new(WINDOW);
        merger.push(2, at(0), vec![line("b", 5, "b1"), line("b", 5, "b2")]);
        merger.push(1, at(0), vec![line("a", 5, "a1"), line("a", 5, "a2")]);
        let all = merger.flush(at(1_000), 100);
        assert_eq!(texts(&all), ["a1", "a2", "b1", "b2"]);
    }

    #[test]
    fn the_same_input_always_merges_to_the_same_output() {
        let run = |first_a: bool| {
            let mut merger = Merger::new(WINDOW);
            let a = (1, vec![line("a", 1, "a1"), line("a", 1, "a2")]);
            let b = (2, vec![line("b", 1, "b1"), line("b", 1, "b2")]);
            let order = if first_a { [a, b] } else { [b, a] };
            for (source, lines) in order {
                merger.push(source, at(0), lines);
            }
            texts(&merger.drain())
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            run(true),
            run(false),
            "the arrival order of the streams does not matter"
        );
    }

    #[test]
    fn a_stream_never_reorders_its_own_lines_whatever_its_clock_does() {
        let mut merger = Merger::new(WINDOW);
        // The pod's clock stepped back between the second and third line.
        merger.push(
            1,
            at(0),
            vec![
                line("a", 100, "a1"),
                line("a", 200, "a2"),
                line("a", 150, "a3"),
            ],
        );
        merger.push(2, at(0), vec![line("b", 160, "b1")]);
        let all = merger.drain();
        assert_eq!(texts(&all), ["a1", "b1", "a2", "a3"], "a3 stays after a2");
    }

    #[test]
    fn a_line_older_than_everything_committed_goes_out_at_the_next_flush() {
        let mut merger = Merger::new(WINDOW);
        merger.push(1, at(0), vec![line("a", 1_000, "a-new")]);
        assert_eq!(texts(&merger.flush(at(300), 100)), ["a-new"]);
        // A pod that joined late brings a backlog from long before: best effort, at once.
        merger.push(2, at(5_000), vec![line("b", 10, "b-backlog")]);
        assert!(
            merger.flush(at(5_100), 100).is_empty(),
            "it waits its own window"
        );
        assert_eq!(texts(&merger.flush(at(5_300), 100)), ["b-backlog"]);
    }

    #[test]
    fn the_waiting_lines_are_bounded() {
        let mut merger = Merger::new(Duration::from_secs(3_600));
        let lines: Vec<LogEntry> = (0..10).map(|i| line("a", i, &format!("l{i}"))).collect();
        merger.push(1, at(0), lines);
        let out = merger.flush(at(1), 4);
        assert_eq!(out.len(), 6, "the oldest are released past the bound");
        assert_eq!(texts(&out[..2]), ["l0", "l1"]);
        assert_eq!(merger.pending(), 4);
    }

    #[test]
    fn overflow_releases_only_the_excess_oldest_lines() {
        let mut merger = Merger::new(Duration::from_secs(3_600));
        merger.push(
            1,
            at(0),
            (0..6).map(|i| line("a", i, &format!("l{i}"))).collect(),
        );
        assert!(merger.overflow(6).is_empty(), "within the bound");
        assert_eq!(texts(&merger.overflow(4)), ["l0", "l1"]);
        assert_eq!(merger.pending(), 4);
    }

    #[test]
    fn drain_releases_everything_in_order() {
        let mut merger = Merger::new(WINDOW);
        merger.push(2, at(0), vec![line("b", 2, "b")]);
        merger.push(1, at(0), vec![line("a", 1, "a")]);
        assert!(merger.has_pending());
        assert_eq!(texts(&merger.drain()), ["a", "b"]);
        assert!(!merger.has_pending());
    }
}
