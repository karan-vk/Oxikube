//! Reading an [`EventBuffer`]: [`EventBuffer::query`] picks the narrowest index the filter
//! allows and walks it newest first; the rest of the filter is checked per event.

use std::collections::{btree_map, btree_set};
use std::iter::Rev;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::event::Event;

use super::{EventBuffer, Slot};
use crate::events::filter::EventFilter;
use crate::events::key::{EventKey, Order};

/// Where a [`Query`] reads positions from.
enum Source<'a> {
    /// The whole ring, newest first.
    Ring(Rev<btree_map::Range<'a, Order, Slot>>),
    /// One index's positions, newest first, resolved through the ring.
    Index(Rev<btree_set::Range<'a, Order>>),
    /// The filter names an object or a namespace that has no events.
    Empty,
}

/// The events of an [`EventBuffer`] that match a filter, newest (by last seen) first. Lazy:
/// taking the first `n` costs `n` matches, not a pass over the buffer.
pub struct Query<'a> {
    source: Source<'a>,
    ring: &'a btree_map::BTreeMap<Order, Slot>,
    filter: &'a EventFilter,
}

impl<'a> Iterator for Query<'a> {
    type Item = &'a Arc<Event>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let event = match &mut self.source {
                Source::Ring(slots) => &slots.next()?.1.event,
                Source::Index(orders) => {
                    let order = orders.next()?;
                    &self.ring.get(order)?.event
                }
                Source::Empty => return None,
            };
            if self.filter.matches(event) {
                return Some(event);
            }
        }
    }
}

impl EventBuffer {
    /// The events matching `filter`, newest first. Uses the involved-object index for an
    /// object filter, the warnings index for "warnings only", the namespace index for a single
    /// namespace, and the ring otherwise; `since` bounds the walk instead of filtering it.
    pub fn query<'a>(&'a self, filter: &'a EventFilter) -> Query<'a> {
        let from: Order = (filter.since_time().unwrap_or(Timestamp::MIN), 0);
        let index = |set: Option<&'a btree_set::BTreeSet<Order>>| {
            set.map_or(Source::Empty, |set| Source::Index(set.range(from..).rev()))
        };
        let source = if let Some(uid) = filter.object_uid() {
            index(self.index.by_object.get(uid).map(|o| &o.orders))
        } else if filter.is_warnings_only() {
            index(Some(&self.index.warnings))
        } else if let Some(namespace) = filter.single_namespace() {
            index(self.index.by_namespace.get(namespace))
        } else {
            Source::Ring(self.ring.range(from..).rev())
        };
        Query {
            source,
            ring: &self.ring,
            filter,
        }
    }

    /// The event with `key`.
    pub fn get(&self, key: &EventKey) -> Option<&Arc<Event>> {
        self.by_key.get(key).map(|order| &self.ring[order].event)
    }

    /// The `Warning` events, newest first (the warnings index alone).
    pub fn warnings(&self) -> impl Iterator<Item = &Arc<Event>> {
        self.index
            .warnings
            .iter()
            .rev()
            .filter_map(|order| self.ring.get(order).map(|slot| &slot.event))
    }

    /// How many events are held about the object with `uid`, and how many of them are
    /// warnings: an `O(1)` lookup (a table's warning badge, E14-S04).
    pub fn object_counts(&self, uid: &str) -> (usize, usize) {
        self.index
            .by_object
            .get(uid)
            .map_or((0, 0), |o| (o.orders.len(), o.warnings))
    }
}
