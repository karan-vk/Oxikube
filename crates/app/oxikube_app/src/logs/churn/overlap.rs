//! [`Overlap`]: drops the lines a reopened stream replays.
//!
//! A reconnect asks for the log from a little before the last line received (`sinceTime`, which
//! the server rounds down to the second), so the first lines of the new stream were read already.
//! The key of a line is its server timestamp and a hash of its text; the set keeps the keys of the
//! last [`RECENT_LINES`] lines (kdash keeps the text of the last 50). While a reconnect replays
//! (until a line newer than the last one received arrives), a line is dropped when its key is in
//! the set or it is older than every remembered line. A line an application repeats keeps a new
//! timestamp, so it is never taken for a replay. Cost per line: one hash of its text and one set
//! insert, whatever the buffer holds.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash as _, Hasher as _};

use jiff::Timestamp;

use crate::logs::LogEntry;

/// Lines whose keys a stream remembers for the overlap of its next reconnect.
pub(crate) const RECENT_LINES: usize = 512;

type Key = (Timestamp, u64);

/// See the [module docs](self).
#[derive(Debug, Default)]
pub(crate) struct Overlap {
    recent: VecDeque<Key>,
    counts: HashMap<Key, u32>,
    /// Set while a reopened stream replays: the newest timestamp received before it.
    replay_until: Option<Timestamp>,
}

impl Overlap {
    /// An overlap that remembers the newest of `entries` (a session reconnected by hand starts
    /// from the lines its buffer kept).
    pub(crate) fn seeded<'a>(entries: impl DoubleEndedIterator<Item = &'a LogEntry>) -> Self {
        let mut overlap = Self::default();
        let newest: Vec<&LogEntry> = entries.rev().take(RECENT_LINES).collect();
        for entry in newest.into_iter().rev() {
            overlap.remember(key(entry));
        }
        overlap
    }

    /// The newest timestamp received, if any line was.
    pub(crate) fn newest(&self) -> Option<Timestamp> {
        self.recent.iter().map(|(ts, _)| *ts).max()
    }

    /// A stream is being reopened: its first lines replay what was received.
    pub(crate) fn begin_replay(&mut self) {
        self.replay_until = self.newest();
    }

    /// Drops the lines of `batch` that were received already; remembers the rest.
    pub(crate) fn filter(&mut self, batch: &mut Vec<LogEntry>) {
        batch.retain(|entry| self.admit(entry));
    }

    fn admit(&mut self, entry: &LogEntry) -> bool {
        let key = key(entry);
        if let Some(until) = self.replay_until {
            if entry.ts > until {
                self.replay_until = None;
            } else if self.counts.contains_key(&key)
                || self
                    .recent
                    .front()
                    .is_some_and(|(oldest, _)| entry.ts < *oldest)
            {
                return false;
            }
        }
        self.remember(key);
        true
    }

    fn remember(&mut self, key: Key) {
        self.recent.push_back(key);
        *self.counts.entry(key).or_default() += 1;
        if self.recent.len() > RECENT_LINES
            && let Some(old) = self.recent.pop_front()
            && let Some(count) = self.counts.get_mut(&old)
        {
            *count -= 1;
            if *count == 0 {
                self.counts.remove(&old);
            }
        }
    }
}

fn key(entry: &LogEntry) -> Key {
    let mut hasher = DefaultHasher::new();
    entry.text.hash(&mut hasher);
    (entry.ts, hasher.finish())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn entry(ms: i64, text: &str) -> LogEntry {
        LogEntry {
            seq: 0,
            ts: Timestamp::from_millisecond(1_760_000_000_000 + ms).unwrap(),
            pod: Arc::from("web-0"),
            container: Arc::from("app"),
            text: Arc::from(text),
            truncated: false,
            level: None,
        }
    }

    fn texts(batch: &[LogEntry]) -> Vec<&str> {
        batch.iter().map(|e| &*e.text).collect()
    }

    #[test]
    fn a_replayed_overlap_is_dropped_and_the_new_lines_kept() {
        let mut overlap = Overlap::default();
        let mut first = vec![entry(0, "a"), entry(10, "b"), entry(10, "c")];
        overlap.filter(&mut first);
        assert_eq!(texts(&first), ["a", "b", "c"]);

        overlap.begin_replay();
        let mut replay = vec![
            entry(0, "a"),
            entry(10, "b"),
            entry(10, "c"),
            entry(20, "d"),
        ];
        overlap.filter(&mut replay);
        assert_eq!(texts(&replay), ["d"]);
        // The replay is over: a repeat of an old text with a new timestamp is a new line.
        let mut later = vec![entry(30, "a")];
        overlap.filter(&mut later);
        assert_eq!(texts(&later), ["a"]);
    }

    #[test]
    fn a_line_missed_inside_the_overlap_is_kept() {
        let mut overlap = Overlap::default();
        overlap.filter(&mut vec![entry(0, "a"), entry(10, "c")]);
        overlap.begin_replay();
        let mut replay = vec![entry(0, "a"), entry(5, "b"), entry(10, "c")];
        overlap.filter(&mut replay);
        assert_eq!(texts(&replay), ["b"], "same second, never seen");
    }

    #[test]
    fn lines_older_than_anything_remembered_are_dropped_while_replaying() {
        let mut overlap = Overlap::default();
        let mut lines: Vec<LogEntry> = (0..(RECENT_LINES as i64 + 10))
            .map(|i| entry(i, &format!("l{i}")))
            .collect();
        overlap.filter(&mut lines);
        overlap.begin_replay();
        let mut replay = vec![entry(1, "l1"), entry(2, "never seen but older")];
        overlap.filter(&mut replay);
        assert!(replay.is_empty());
    }

    #[test]
    fn the_memory_is_bounded() {
        let mut overlap = Overlap::default();
        for i in 0..5_000 {
            overlap.filter(&mut vec![entry(i, "same text")]);
        }
        assert_eq!(overlap.recent.len(), RECENT_LINES);
        assert!(overlap.counts.len() <= RECENT_LINES);
    }

    #[test]
    fn a_seeded_overlap_drops_what_the_buffer_holds() {
        let kept = [entry(0, "a"), entry(10, "b")];
        let mut overlap = Overlap::seeded(kept.iter());
        overlap.begin_replay();
        let mut replay = vec![entry(10, "b"), entry(11, "c")];
        overlap.filter(&mut replay);
        assert_eq!(texts(&replay), ["c"]);
    }
}
