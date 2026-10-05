//! Dropping the lines a reconnect replays.
//!
//! After a reconnect the server is asked for everything since a little before the last line
//! seen (the overlap), so nothing written during the gap is missed. The overlap replays lines
//! that were already delivered; [`Dedup`] recognises them by a hash of `(timestamp, text)`.
//!
//! Matching on the timestamp as well as the text means a line the application genuinely
//! repeats (`"retrying..."` every second) is never mistaken for a replay: only the exact same
//! kubelet timestamp matches. Matching is a multiset: if two identical lines carry the same
//! timestamp, a replay of both drops both, and a replay of one drops one.
//!
//! Outside a replay nothing is dropped, so a stream that happens to repeat a `(timestamp,
//! text)` pair keeps both.

use std::collections::{HashMap, VecDeque};

use jiff::Timestamp;

/// Remembers the last `cap` delivered lines and filters the replay after a reconnect.
pub(crate) struct Dedup {
    cap: usize,
    /// Keys of the delivered lines, oldest first.
    ring: VecDeque<u64>,
    /// How many of each key are in `ring`.
    counts: HashMap<u64, u32>,
    /// Latest timestamp delivered.
    newest: Option<Timestamp>,
    /// While replaying: the newest timestamp at the time of the reconnect. A line after it
    /// is new, and ends the replay.
    boundary: Option<Timestamp>,
    /// While replaying: how many of each key the replay has already matched.
    matched: HashMap<u64, u32>,
}

impl Dedup {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            ring: VecDeque::new(),
            counts: HashMap::new(),
            newest: None,
            boundary: None,
            matched: HashMap::new(),
        }
    }

    /// The latest timestamp delivered, from which a reconnect resumes.
    pub(crate) fn newest(&self) -> Option<Timestamp> {
        self.newest
    }

    /// Marks the start of a reconnect: lines up to [`newest`](Self::newest) that were
    /// delivered already are dropped until the first newer line arrives.
    pub(crate) fn begin_replay(&mut self) {
        self.boundary = self.newest;
        self.matched.clear();
    }

    /// Whether to deliver the line. `ts` is the line's timestamp (a line without one takes
    /// the newest seen, so it is never older than what was delivered).
    pub(crate) fn admit(&mut self, ts: Timestamp, key: u64) -> bool {
        if let Some(boundary) = self.boundary {
            if ts > boundary {
                self.boundary = None;
                self.matched.clear();
            } else if let Some(&delivered) = self.counts.get(&key) {
                let matched = self.matched.entry(key).or_insert(0);
                if *matched < delivered {
                    *matched += 1;
                    // A replay of a line already delivered: drop it, and keep the record.
                    return false;
                }
            }
        }
        self.record(key);
        if self.newest.is_none_or(|n| ts > n) {
            self.newest = Some(ts);
        }
        true
    }

    fn record(&mut self, key: u64) {
        *self.counts.entry(key).or_insert(0) += 1;
        self.ring.push_back(key);
        if self.ring.len() > self.cap {
            if let Some(old) = self.ring.pop_front() {
                if let Some(count) = self.counts.get_mut(&old) {
                    *count -= 1;
                    if *count == 0 {
                        self.counts.remove(&old);
                    }
                }
            }
        }
    }
}
