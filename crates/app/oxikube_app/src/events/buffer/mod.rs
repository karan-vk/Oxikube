//! [`EventBuffer`]: one cluster's events in a bounded ring, de-duplicated and indexed.
//!
//! The ring is ordered by ([`last seen`](oxikube_domain::event::Event::last_seen), arrival), so
//! the oldest event is its first entry and a query walks it backwards, newest first. A second
//! delivery of an event (same [`EventKey`]) replaces the first in place: count, last seen and
//! message move to the new state and the event moves to the recent end. At capacity, a new event
//! evicts the oldest; an event older than everything held is not stored. Every insert and removal
//! updates the ring, the key map and every index in the same step, so they cannot disagree.
//!
//! | Structure | Holds | Cost |
//! |---|---|---|
//! | `ring` | `Order -> (key, Arc<Event>)` | insert, evict, update `O(log n)` |
//! | `by_key` | `EventKey -> Order` (the dedupe map) | `O(1)` |
//! | `index.by_object` | involved-object uid `->` positions, warning count | `O(log k)` per change, `O(1)` count |
//! | `index.by_namespace` | event namespace `->` positions | `O(log k)` per change |
//! | `index.warnings` | positions of `Warning` events | `O(log w)` per change |

mod index;
mod query;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use oxikube_domain::event::Event;
use oxikube_ports::Delta;

use super::changes::{Change, ChangeKind};
use super::key::{EventKey, Order, seen_at};
use index::Indexes;

pub use query::Query;

/// An event in the ring with its key.
#[derive(Debug)]
struct Slot {
    key: EventKey,
    event: Arc<Event>,
}

/// One cluster's events, see the [module docs](self).
#[derive(Debug)]
pub struct EventBuffer {
    capacity: usize,
    seq: u64,
    ring: BTreeMap<Order, Slot>,
    by_key: HashMap<EventKey, Order>,
    index: Indexes,
    evicted: u64,
}

/// What [`EventBuffer::upsert`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upserted {
    /// A new event was stored (possibly evicting the oldest).
    Added,
    /// A stored event moved to its new state.
    Updated,
    /// The same state was already stored.
    Unchanged,
    /// The buffer is full and the event is older than everything in it: not stored.
    Dropped,
}

impl EventBuffer {
    /// An empty buffer holding at most `capacity` events (at least one).
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            seq: 0,
            ring: BTreeMap::new(),
            by_key: HashMap::new(),
            index: Indexes::default(),
            evicted: 0,
        }
    }

    /// A buffer of `capacity` holding the newest of `events` (a relist).
    pub fn from_events(capacity: usize, events: impl IntoIterator<Item = Event>) -> Self {
        let mut buffer = Self::new(capacity);
        let mut sink = Vec::new();
        for event in events {
            buffer.upsert_into(event, &mut sink);
            sink.clear();
        }
        buffer
    }

    /// The most events held.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Events held.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// Whether no event is held.
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// Events evicted (or not stored) to stay within the capacity, since the buffer was made.
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// Stores `event`, or moves the stored event with its key to this state.
    pub fn upsert(&mut self, event: Event) -> Upserted {
        self.upsert_into(event, &mut Vec::new())
    }

    /// Removes the event with `key`.
    pub fn remove(&mut self, key: &EventKey) -> Option<Arc<Event>> {
        let order = self.by_key.remove(key)?;
        let slot = self.ring.remove(&order)?;
        self.index.remove(order, &slot.event);
        Some(slot.event)
    }

    /// Applies one feed delta. A `Restarted` replaces every event.
    pub fn apply(&mut self, delta: Delta<Event>) {
        self.apply_into(delta, &mut Vec::new());
    }

    /// Sets the capacity (at least one): shrinking evicts the oldest at once, growing keeps
    /// every event.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.set_capacity_into(capacity, &mut Vec::new());
    }

    pub(crate) fn set_capacity_into(&mut self, capacity: usize, out: &mut Vec<Change>) {
        self.capacity = capacity.max(1);
        while self.ring.len() > self.capacity {
            self.evict_oldest(out);
        }
    }

    /// Removes every event (the evicted count stays).
    pub fn clear(&mut self) {
        self.ring.clear();
        self.by_key.clear();
        self.index.clear();
    }

    /// Applies `delta`, recording what changed in `out`. Returns `true` when it was a
    /// `Restarted` (the caller tells subscribers to read everything again).
    pub(crate) fn apply_into(&mut self, delta: Delta<Event>, out: &mut Vec<Change>) -> bool {
        match delta {
            Delta::Applied(event) => {
                self.upsert_into(event, out);
                false
            }
            Delta::Deleted(event) => {
                let key = EventKey::of(&event);
                if let Some(event) = self.remove(&key) {
                    out.push(change(key, ChangeKind::Removed, event, None));
                }
                false
            }
            Delta::Restarted(events) => {
                self.clear();
                for event in events {
                    self.upsert_into(event, out);
                }
                out.clear();
                true
            }
        }
    }

    pub(crate) fn upsert_into(&mut self, event: Event, out: &mut Vec<Change>) -> Upserted {
        let key = EventKey::of(&event);
        let previous = match self.by_key.get(&key) {
            Some(order) => {
                let slot = &self.ring[order];
                if *slot.event == event {
                    return Upserted::Unchanged;
                }
                let previous = slot.event.clone();
                self.remove(&key);
                Some(previous)
            }
            None => None,
        };
        self.seq += 1;
        let order = (seen_at(&event), self.seq);
        if previous.is_none() && self.ring.len() >= self.capacity {
            if self
                .ring
                .first_key_value()
                .is_some_and(|(oldest, _)| order < *oldest)
            {
                self.evicted += 1;
                return Upserted::Dropped;
            }
            self.evict_oldest(out);
        }
        let event = Arc::new(event);
        self.index.insert(order, &event);
        self.by_key.insert(key.clone(), order);
        self.ring.insert(
            order,
            Slot {
                key: key.clone(),
                event: event.clone(),
            },
        );
        let kind = if previous.is_some() {
            ChangeKind::Updated
        } else {
            ChangeKind::Added
        };
        out.push(change(key, kind, event, previous));
        match kind {
            ChangeKind::Updated => Upserted::Updated,
            _ => Upserted::Added,
        }
    }

    fn evict_oldest(&mut self, out: &mut Vec<Change>) {
        if let Some((order, slot)) = self.ring.pop_first() {
            self.by_key.remove(&slot.key);
            self.index.remove(order, &slot.event);
            self.evicted += 1;
            out.push(change(slot.key, ChangeKind::Removed, slot.event, None));
        }
    }
}

fn change(
    key: EventKey,
    kind: ChangeKind,
    event: Arc<Event>,
    previous: Option<Arc<Event>>,
) -> Change {
    Change {
        key,
        kind,
        event,
        previous,
    }
}

#[cfg(test)]
impl EventBuffer {
    /// Every event held, newest first, read off the ring alone.
    pub(crate) fn ring_events(&self) -> Vec<Arc<Event>> {
        self.ring.values().rev().map(|s| s.event.clone()).collect()
    }

    /// Panics unless the key map and every index agree with a brute-force scan of the ring.
    pub(crate) fn assert_consistent(&self) {
        use std::collections::BTreeSet;
        assert!(self.ring.len() <= self.capacity, "over capacity");
        assert_eq!(self.by_key.len(), self.ring.len(), "by_key size");
        let mut objects: HashMap<Arc<str>, (BTreeSet<Order>, usize)> = HashMap::new();
        let mut namespaces: HashMap<Arc<str>, BTreeSet<Order>> = HashMap::new();
        let mut warnings = BTreeSet::new();
        for (order, slot) in &self.ring {
            assert_eq!(
                self.by_key.get(&slot.key),
                Some(order),
                "by_key points elsewhere"
            );
            assert_eq!(slot.key, EventKey::of(&slot.event), "slot key");
            if let Some(uid) = &slot.event.regarding_uid {
                let entry = objects.entry(uid.clone()).or_default();
                entry.0.insert(*order);
                entry.1 += usize::from(slot.event.is_warning());
            }
            if let Some(ns) = &slot.event.namespace {
                namespaces.entry(ns.clone()).or_default().insert(*order);
            }
            if slot.event.is_warning() {
                warnings.insert(*order);
            }
        }
        let indexed: HashMap<Arc<str>, (BTreeSet<Order>, usize)> = self
            .index
            .by_object
            .iter()
            .map(|(k, v)| (k.clone(), (v.orders.clone(), v.warnings)))
            .collect();
        assert_eq!(indexed, objects, "by_object");
        assert_eq!(self.index.by_namespace, namespaces, "by_namespace");
        assert_eq!(self.index.warnings, warnings, "warnings");
    }
}
