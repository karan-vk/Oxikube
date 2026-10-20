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
use crate::events::object::ObjectIdentity;

/// Where a [`Query`] reads positions from.
enum Source<'a> {
    /// The whole ring, newest first.
    Ring(Rev<btree_map::Range<'a, Order, Slot>>),
    /// One index's positions, newest first, resolved through the ring.
    Index(Rev<btree_set::Range<'a, Order>>),
    /// Two indexes' positions, newest first: the events about one object, the ones that carry
    /// its uid and the ones that gave no uid and named it.
    Union(Union<'a>),
    /// The filter names an object or a namespace that has no events.
    Empty,
}

/// The positions of two indexes, newest first: the events about one object, the ones that
/// carry its uid and the ones that gave no uid and named it.
///
/// The two sets never share an event (an event has a uid or it has not), so the merge never
/// repeats one.
struct Union<'a> {
    by_uid: Rev<btree_set::Range<'a, Order>>,
    by_name: Rev<btree_set::Range<'a, Order>>,
    uid_head: Option<&'a Order>,
    name_head: Option<&'a Order>,
}

impl<'a> Union<'a> {
    fn new(
        by_uid: Rev<btree_set::Range<'a, Order>>,
        by_name: Rev<btree_set::Range<'a, Order>>,
    ) -> Self {
        Self {
            by_uid,
            by_name,
            uid_head: None,
            name_head: None,
        }
    }
}

impl<'a> Iterator for Union<'a> {
    type Item = &'a Order;

    fn next(&mut self) -> Option<&'a Order> {
        if self.uid_head.is_none() {
            self.uid_head = self.by_uid.next();
        }
        if self.name_head.is_none() {
            self.name_head = self.by_name.next();
        }
        match (self.uid_head, self.name_head) {
            (Some(uid), Some(name)) => {
                if name > uid {
                    self.name_head = self.by_name.next();
                    Some(name)
                } else {
                    self.uid_head = self.by_uid.next();
                    Some(uid)
                }
            }
            (Some(uid), None) => {
                self.uid_head = self.by_uid.next();
                Some(uid)
            }
            (None, Some(name)) => {
                self.name_head = self.by_name.next();
                Some(name)
            }
            (None, None) => None,
        }
    }
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
                Source::Union(orders) => {
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
    /// object filter (by uid, and by name for the events that gave no uid), the warnings index
    /// for "warnings only", the namespace index for a single namespace, and the ring otherwise;
    /// `since` bounds the walk instead of filtering it.
    pub fn query<'a>(&'a self, filter: &'a EventFilter) -> Query<'a> {
        let from: Order = (filter.since_time().unwrap_or(Timestamp::MIN), 0);
        let index = |set: Option<&'a btree_set::BTreeSet<Order>>| {
            set.map_or(Source::Empty, |set| Source::Index(set.range(from..).rev()))
        };
        let source = if let Some(uid) = filter.object_uid() {
            let by_uid = self.index.by_object.get(uid).map(|o| &o.orders);
            let by_name = filter
                .object_identity()
                .and_then(|identity| self.index.by_named.get(identity))
                .map(|o| &o.orders);
            match (by_uid, by_name) {
                (Some(by_uid), Some(by_name)) => Source::Union(Union::new(
                    by_uid.range(from..).rev(),
                    by_name.range(from..).rev(),
                )),
                (Some(by_uid), None) => Source::Index(by_uid.range(from..).rev()),
                (None, Some(by_name)) => Source::Index(by_name.range(from..).rev()),
                (None, None) => Source::Empty,
            }
        } else if let Some(identity) = filter.object_identity() {
            index(self.index.by_named.get(identity).map(|o| &o.orders))
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
    /// warnings: an `O(1)` lookup (a table's warning badge, E14-S04). The events that gave no
    /// uid are counted too, when the object's `identity` is known.
    pub fn object_counts(&self, uid: &str, identity: Option<&ObjectIdentity>) -> (usize, usize) {
        let by_uid = self.index.by_object.get(uid);
        let by_named = identity.and_then(|identity| self.index.by_named.get(identity));
        match (by_uid, by_named) {
            (Some(by_uid), Some(by_named)) => (
                by_uid.orders.len() + by_named.orders.len(),
                by_uid.warnings + by_named.warnings,
            ),
            (Some(by_uid), None) => (by_uid.orders.len(), by_uid.warnings),
            (None, Some(by_named)) => (by_named.orders.len(), by_named.warnings),
            (None, None) => (0, 0),
        }
    }
}
