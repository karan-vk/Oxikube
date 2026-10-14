//! [`EventSort`]: the order of the rows. Last seen, newest first, unless the user clicks a header.

use std::cmp::Ordering;

use jiff::Timestamp;
use oxikube_domain::event::{Event, EventType};

use super::row::{EventColumn, EventRow};

/// Which column the rows are ordered by, and which way. Every order is total: equal values fall
/// back to the order of arrival, so a row has exactly one place and is found again by a binary
/// search when it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventSort {
    /// The column.
    pub column: EventColumn,
    /// Largest (newest, most, last in the alphabet) first.
    pub descending: bool,
}

impl Default for EventSort {
    /// Last seen, newest first: the live tail's order.
    fn default() -> Self {
        Self {
            column: EventColumn::LastSeen,
            descending: true,
        }
    }
}

impl EventSort {
    /// Whether this is the order the service returns a query in (newest last seen first, later
    /// arrivals first), so a fresh list needs no sort.
    pub fn is_service_order(self) -> bool {
        self == Self::default()
    }

    /// Where row `a` goes relative to row `b`.
    pub fn compare(self, a: &EventRow, b: &EventRow) -> Ordering {
        self.order((a.id, &a.event), (b.id, &b.event))
    }

    /// Where the event with arrival number `a.0` goes relative to `b`'s: what a binary search
    /// for a row compares with, without building one.
    pub fn order(self, a: (u64, &Event), b: (u64, &Event)) -> Ordering {
        let order = compare_by(self.column, a.1, b.1).then(a.0.cmp(&b.0));
        if self.descending {
            order.reverse()
        } else {
            order
        }
    }
}

fn compare_by(column: EventColumn, a: &Event, b: &Event) -> Ordering {
    match column {
        EventColumn::Type => type_rank(a.event_type).cmp(&type_rank(b.event_type)),
        EventColumn::Reason => a.reason.cmp(&b.reason),
        EventColumn::Namespace => a.namespace.cmp(&b.namespace),
        EventColumn::Object => (&a.regarding.gvk.kind, &a.regarding.name)
            .cmp(&(&b.regarding.gvk.kind, &b.regarding.name)),
        EventColumn::Source => a.reporting_component.cmp(&b.reporting_component),
        EventColumn::Count => a.count.cmp(&b.count),
        // Ascending age is the youngest first: the latest first sighting.
        EventColumn::Age => first_seen(b).cmp(&first_seen(a)),
        EventColumn::LastSeen => last_seen(a).cmp(&last_seen(b)),
        EventColumn::Message => a.message.cmp(&b.message),
    }
}

fn type_rank(event_type: EventType) -> u8 {
    match event_type {
        EventType::Normal => 0,
        EventType::Other => 1,
        EventType::Warning => 2,
    }
}

/// The time an event is ordered by in the service: `last_seen`, else `first_seen`.
pub(crate) fn last_seen(event: &Event) -> Timestamp {
    event
        .last_seen
        .or(event.first_seen)
        .unwrap_or(Timestamp::MIN)
}

fn first_seen(event: &Event) -> Timestamp {
    event
        .first_seen
        .or(event.last_seen)
        .unwrap_or(Timestamp::MIN)
}
