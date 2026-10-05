//! [`EventRing`]: the bounded, de-duplicating store behind a feed.
//!
//! It holds at most `capacity` events, one per [`Key`], and says what changed as
//! [`Delta`]s: the consumer that folds them holds at most `capacity` events too.
//!
//! # De-duplication
//!
//! Both event APIs serve the same stored object, so the same event arrives once per API
//! (and again after every update). Two events are the same event when they have the same
//! `metadata.uid`; an event without one (not seen from a real server) is keyed on
//! (regarding object, reason, message). The ring then keeps one entry and applies this rule:
//!
//! * an update from the API the entry came from replaces it when anything changed;
//! * an update from the *other* API replaces it only when it is strictly newer, by
//!   (`last_seen`, `count`). A tie keeps the entry, so the two views of one update never
//!   alternate and never produce a second delta.
//!
//! # Who holds what
//!
//! An entry remembers which APIs delivered it. A server-side delete removes it outright (both
//! APIs see the same delete). A relist of one API retires an entry only if that API was the
//! only one to deliver it, so two APIs that disagree for a moment do not erase each other's
//! events; a namespace's relist only touches that namespace's events.
//!
//! # Eviction
//!
//! At capacity, a new event evicts the entry with the oldest `last_seen` (insertion order
//! breaks ties; an event without a time is the oldest). The eviction is sent as a
//! `Deleted` delta before the new event's `Applied`, so a consumer never holds more than
//! `capacity`. A new event older than everything held is not stored at all. Both count as
//! evicted ([`EventRing::evicted`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::event::Event;
use oxikube_domain::ids::ResourceRef;
use oxikube_ports::Delta;

use super::config::EventApi;

/// Identity of one event across both APIs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum Key {
    /// `metadata.uid`.
    Uid(Arc<str>),
    /// (regarding object, reason, message), for an event that has no uid.
    Content(Box<(ResourceRef, Arc<str>, String)>),
}

impl Key {
    pub(super) fn of(event: &Event) -> Self {
        match &event.uid {
            Some(uid) => Self::Uid(uid.clone()),
            None => Self::Content(Box::new((
                event.regarding.clone(),
                event.reason.clone(),
                event.message.clone(),
            ))),
        }
    }
}

/// Eviction order: last-seen time, then arrival.
type Order = (Timestamp, u64);

struct Entry {
    event: Event,
    /// The API whose view `event` is.
    origin: EventApi,
    /// The namespace watch that delivered it (`None` for a cluster-wide watch).
    namespace: Option<Arc<str>>,
    /// The APIs that delivered it ([`EventApi::bit`]).
    holders: u8,
    order: Order,
}

/// A bounded set of events. See the [module docs](self).
pub(super) struct EventRing {
    capacity: usize,
    entries: HashMap<Key, Entry>,
    order: BTreeMap<Order, Key>,
    next_seq: u64,
    evicted: u64,
}

impl EventRing {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: HashMap::new(),
            order: BTreeMap::new(),
            next_seq: 0,
            evicted: 0,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Events dropped to stay within capacity so far.
    pub(super) fn evicted(&self) -> u64 {
        self.evicted
    }

    /// The held events, oldest last-seen first.
    pub(super) fn snapshot(&self) -> Vec<Event> {
        self.order
            .values()
            .filter_map(|key| self.entries.get(key))
            .map(|entry| entry.event.clone())
            .collect()
    }

    fn order_of(&mut self, event: &Event) -> Order {
        self.next_seq += 1;
        (event.last_seen.unwrap_or(Timestamp::MIN), self.next_seq)
    }

    /// Adds or updates the event `key` as seen through `api` by the watch of `namespace`,
    /// pushing the resulting deltas onto `out`.
    pub(super) fn upsert(
        &mut self,
        key: Key,
        api: EventApi,
        namespace: &Option<Arc<str>>,
        event: Event,
        out: &mut Vec<Delta<Event>>,
    ) {
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.holders |= api.bit();
            let newer = (event.last_seen, event.count) > (entry.event.last_seen, entry.event.count);
            if (entry.origin == api || newer) && event != entry.event {
                let order = self.order_of(&event);
                let entry = self.entries.get_mut(&key).expect("entry checked above");
                self.order.remove(&entry.order);
                self.order.insert(order, key);
                entry.order = order;
                entry.origin = api;
                entry.event = event.clone();
                out.push(Delta::Applied(event));
            }
            return;
        }
        if self.entries.len() >= self.capacity && !self.make_room(&event, out) {
            return;
        }
        let order = self.order_of(&event);
        self.order.insert(order, key.clone());
        self.entries.insert(
            key,
            Entry {
                event: event.clone(),
                origin: api,
                namespace: namespace.clone(),
                holders: api.bit(),
                order,
            },
        );
        out.push(Delta::Applied(event));
    }

    /// Evicts the oldest entry for `incoming`. `false` when `incoming` is older than every
    /// entry, so it is the one to drop.
    fn make_room(&mut self, incoming: &Event, out: &mut Vec<Delta<Event>>) -> bool {
        self.evicted += 1;
        let Some((&(oldest, _), _)) = self.order.first_key_value() else {
            return true;
        };
        if incoming.last_seen.unwrap_or(Timestamp::MIN) < oldest {
            return false;
        }
        if let Some((_, key)) = self.order.pop_first() {
            if let Some(entry) = self.entries.remove(&key) {
                out.push(Delta::Deleted(entry.event));
            }
        }
        true
    }

    /// Removes the event `key`, if held.
    pub(super) fn remove(&mut self, key: &Key, out: &mut Vec<Delta<Event>>) {
        if let Some(entry) = self.entries.remove(key) {
            self.order.remove(&entry.order);
            out.push(Delta::Deleted(entry.event));
        }
    }

    /// Retires what the relist of `api` in `namespace` did not return (`seen`): the server
    /// dropped it while the watch was down. An event that the other API also delivered stays.
    pub(super) fn retire_unseen(
        &mut self,
        api: EventApi,
        namespace: &Option<Arc<str>>,
        seen: &HashSet<Key>,
        out: &mut Vec<Delta<Event>>,
    ) {
        let mut gone = Vec::new();
        for (key, entry) in &mut self.entries {
            if entry.namespace == *namespace && !seen.contains(key) {
                entry.holders &= !api.bit();
                if entry.holders == 0 {
                    gone.push(key.clone());
                }
            }
        }
        for key in gone {
            self.remove(&key, out);
        }
    }
}
