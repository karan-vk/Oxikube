//! [`Tracker`]: which of a cluster's events are news, so the centre notifies about what happens
//! while it watches and never about the backlog a list or a relist replays.
//!
//! * An event the tracker has counted before is news when its count went up, by the difference.
//! * The first list is the backlog: none of it is news.
//! * An event it has not seen is news when it was last seen after the tracker started (less a
//!   small allowance for the cluster's clock) when it arrives as an addition, and after the latest
//!   event seen *before* that reset when it arrives in a later one (a relist after a reconnect
//!   replays what was already told, and adds only what happened during the gap).
//! * An empty reset (the feed stopped: a disconnect) changes nothing, so the reconnect's relist
//!   is recognised.
//!
//! The counts held are those of the events the cluster's buffer holds (a reset trims the rest),
//! so the tracker is bounded by `events.buffer`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::event::Event;

use crate::events::EventKey;

/// How far behind the local clock a cluster's event timestamps may be and still count as new
/// when the tracker starts.
pub(crate) const CLOCK_ALLOWANCE: SignedDuration = SignedDuration::from_secs(30);

/// One cluster's view of which events were told. See the [module docs](self).
#[derive(Debug)]
pub(crate) struct Tracker {
    since: Timestamp,
    latest: Timestamp,
    counts: HashMap<EventKey, u32>,
    /// Something was seen: a reset from now on is a relist, not the backlog.
    seen: bool,
}

impl Tracker {
    /// A tracker started at `now`.
    pub(crate) fn new(now: Timestamp) -> Self {
        let since = now.checked_sub(CLOCK_ALLOWANCE).unwrap_or(now);
        Self {
            since,
            latest: since,
            counts: HashMap::new(),
            seen: false,
        }
    }

    /// Counts `event`, an addition or a change; returns how many of its occurrences are news
    /// (`None`: none).
    pub(crate) fn observe(&mut self, event: &Event) -> Option<u32> {
        self.count(event, None)
    }

    /// Counts `event`. An event not seen before is judged against `relist_floor` when it comes
    /// from a reset (the latest time told before that reset), else against the start.
    fn count(&mut self, event: &Event, relist_floor: Option<Timestamp>) -> Option<u32> {
        let key = EventKey::of(event);
        let at = seen_at(event);
        let count = event.count.max(1);
        let news = match (self.counts.get(&key), relist_floor) {
            (Some(&before), _) => count.checked_sub(before).filter(|more| *more > 0),
            (None, Some(floor)) => (at > floor).then_some(1),
            (None, None) => {
                let timeless = event.last_seen.is_none() && event.first_seen.is_none();
                (timeless || at > self.since).then_some(1)
            }
        };
        self.counts.insert(key, count);
        if at > self.latest {
            self.latest = at;
        }
        self.seen = true;
        news
    }

    /// Reads a reset: the events the cluster's buffer now holds, in any order (the feed gives
    /// them newest first). Returns the news among them, in the order given. Every unseen event
    /// is judged against the latest time told *before* the reset, so a gap holding several new
    /// events tells them all. An empty reset is ignored (see the [module docs](self)).
    pub(crate) fn reset(&mut self, events: &[Arc<Event>]) -> Vec<(Arc<Event>, u32)> {
        if events.is_empty() {
            return Vec::new();
        }
        let backlog = !self.seen;
        let floor = self.latest;
        let news = events
            .iter()
            .filter_map(|event| self.count(event, Some(floor)).map(|n| (event.clone(), n)))
            .filter(|_| !backlog)
            .collect();
        let held: HashSet<EventKey> = events.iter().map(|e| EventKey::of(e)).collect();
        self.counts.retain(|key, _| held.contains(key));
        news
    }

    /// Forgets an event the buffer no longer holds.
    pub(crate) fn forget(&mut self, key: &EventKey) {
        self.counts.remove(key);
    }

    /// How many events are counted.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.counts.len()
    }
}

/// The time an event is ordered by in the events buffer: last seen, else first seen.
fn seen_at(event: &Event) -> Timestamp {
    event
        .last_seen
        .or(event.first_seen)
        .unwrap_or(Timestamp::MIN)
}
