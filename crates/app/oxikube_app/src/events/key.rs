//! [`EventKey`]: what makes two deliveries the same event, and the eviction [`Order`].

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::event::Event;
use oxikube_domain::ids::ResourceRef;

/// The identity of one event in a buffer: its `metadata.uid`, or, for an event without one
/// (not seen from a real server), its (regarding object, reason, message). The same rule as the
/// adapter's ring, so the two agree on what a duplicate is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventKey(Repr);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Repr {
    Uid(Arc<str>),
    Content(Arc<(ResourceRef, Arc<str>, String)>),
}

impl EventKey {
    /// The key of `event`.
    pub fn of(event: &Event) -> Self {
        match &event.uid {
            Some(uid) => Self(Repr::Uid(uid.clone())),
            None => Self(Repr::Content(Arc::new((
                event.regarding.clone(),
                event.reason.clone(),
                event.message.clone(),
            )))),
        }
    }

    /// The event's uid, when it has one.
    pub fn uid(&self) -> Option<&str> {
        match &self.0 {
            Repr::Uid(uid) => Some(uid),
            Repr::Content(_) => None,
        }
    }
}

/// Where an event sits in a buffer: its last-seen time, then its arrival. Ascending is oldest
/// first; an event without a time is the oldest (as in the adapter's ring).
pub(crate) type Order = (Timestamp, u64);

/// The time an event is ordered by: `last_seen`, else `first_seen`, else the oldest time.
pub(crate) fn seen_at(event: &Event) -> Timestamp {
    event
        .last_seen
        .or(event.first_seen)
        .unwrap_or(Timestamp::MIN)
}
