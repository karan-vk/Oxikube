//! [`EventsChanged`]: what a subscriber is told, and how a burst of buffer changes coalesces
//! into one of them per poll.

use std::sync::Arc;

use indexmap::IndexMap;
use indexmap::map::Entry;
use oxikube_domain::event::Event;

use super::filter::EventFilter;
use super::key::EventKey;
use super::state::EventsState;

/// Pending changes a subscriber may hold before they collapse into a reset: past this many, one
/// re-query is cheaper than replaying them.
const MAX_PENDING: usize = 4_096;

/// The changes to a cluster's events since a subscriber last polled, limited to the events its
/// filter matches. A subscriber that polls once per frame gets at most one of these per frame,
/// however many events arrived (docs/PERFORMANCE.md rule 2).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventsChanged {
    /// The events were replaced as a whole (the first item of a subscription, a relist, a
    /// reconnect, a new filter, a burst too large to list): read them again with a query.
    /// The lists below are then empty.
    pub reset: bool,
    /// Events that now match and did not before, oldest first.
    pub added: Vec<Arc<Event>>,
    /// Events that matched before and still match, in their newest state (a higher count, a
    /// later last seen, a new message), oldest first.
    pub updated: Vec<Arc<Event>>,
    /// Events that matched before and are gone: deleted, evicted, or no longer matching.
    pub removed: Vec<EventKey>,
    /// The feed's new state, when it changed since the last poll (warming, ready, retrying,
    /// failed).
    pub state: Option<EventsState>,
}

impl EventsChanged {
    /// Whether there is nothing to tell.
    pub fn is_empty(&self) -> bool {
        !self.reset
            && self.added.is_empty()
            && self.updated.is_empty()
            && self.removed.is_empty()
            && self.state.is_none()
    }
}

/// What happened to one event in a buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    Added,
    Updated,
    Removed,
}

/// One change of a buffer: the event's key, what happened, its state now (its last state for a
/// removal) and, for an update, its state before.
#[derive(Debug, Clone)]
pub(crate) struct Change {
    pub(crate) key: EventKey,
    pub(crate) kind: ChangeKind,
    pub(crate) event: Arc<Event>,
    pub(crate) previous: Option<Arc<Event>>,
}

/// One subscriber's changes not yet polled, merged per event.
#[derive(Debug, Default)]
pub(crate) struct Pending {
    reset: bool,
    changes: IndexMap<EventKey, (ChangeKind, Arc<Event>)>,
    state: Option<EventsState>,
}

impl Pending {
    /// A pending reset (what a new subscriber gets first).
    pub(crate) fn reset() -> Self {
        Self {
            reset: true,
            ..Self::default()
        }
    }

    pub(crate) fn set_state(&mut self, state: EventsState) {
        self.state = Some(state);
    }

    pub(crate) fn set_reset(&mut self) {
        self.reset = true;
        self.changes.clear();
    }

    pub(crate) fn is_empty(&self) -> bool {
        !self.reset && self.changes.is_empty() && self.state.is_none()
    }

    /// Adds the buffer's `changes`, seen through `filter`: an update that makes an event match is
    /// an addition for this subscriber, one that makes it stop matching a removal.
    pub(crate) fn push(&mut self, changes: &[Change], filter: &EventFilter) {
        if self.reset {
            return;
        }
        for change in changes {
            let now = change.kind != ChangeKind::Removed && filter.matches(&change.event);
            let before = match change.kind {
                ChangeKind::Added => false,
                ChangeKind::Updated => change
                    .previous
                    .as_deref()
                    .is_some_and(|p| filter.matches(p)),
                ChangeKind::Removed => filter.matches(&change.event),
            };
            let kind = match (before, now) {
                (false, false) => continue,
                (false, true) => ChangeKind::Added,
                (true, true) => ChangeKind::Updated,
                (true, false) => ChangeKind::Removed,
            };
            self.merge(change.key.clone(), kind, change.event.clone());
        }
        if self.changes.len() > MAX_PENDING {
            self.set_reset();
        }
    }

    fn merge(&mut self, key: EventKey, kind: ChangeKind, event: Arc<Event>) {
        use ChangeKind::{Added, Removed, Updated};
        match self.changes.entry(key) {
            Entry::Vacant(slot) => {
                slot.insert((kind, event));
            }
            Entry::Occupied(mut slot) => match (slot.get().0, kind) {
                // Added and gone before anyone looked: nothing to tell.
                (Added, Removed) => {
                    slot.shift_remove();
                }
                (Added, _) => *slot.get_mut() = (Added, event),
                // Gone and back: the subscriber still holds the old state.
                (Removed, Added | Updated) => *slot.get_mut() = (Updated, event),
                (_, kind) => *slot.get_mut() = (kind, event),
            },
        }
    }

    /// The changes as one item, leaving nothing pending.
    pub(crate) fn take(&mut self) -> EventsChanged {
        let mut out = EventsChanged {
            reset: std::mem::take(&mut self.reset),
            state: self.state.take(),
            ..EventsChanged::default()
        };
        for (key, (kind, event)) in std::mem::take(&mut self.changes) {
            match kind {
                ChangeKind::Added => out.added.push(event),
                ChangeKind::Updated => out.updated.push(event),
                ChangeKind::Removed => out.removed.push(key),
            }
        }
        out
    }
}
