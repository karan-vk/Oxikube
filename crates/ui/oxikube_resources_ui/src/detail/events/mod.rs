//! The Events tab's rows: the events about the object, as the app's events index lists them.
//!
//! The list is the [`ObjectEventsFeed`](oxikube_app::ObjectEventsFeed) of the object (the
//! `EventService`'s one cluster-wide feed read by involved-object uid, newest first): the tab
//! opens no watch and makes no API call of its own.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::event::{Event, EventType};

/// Most events kept.
pub const MAX_EVENTS: usize = oxikube_app::warning_badges::MAX_OBJECT_EVENTS;

/// One event as the tab lists it: the columns of the events view (Type, Reason, Message, Count,
/// Age) and who reported it.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    /// `Normal` or `Warning`.
    pub kind: EventType,
    /// The short reason, for example `BackOff`.
    pub reason: Arc<str>,
    /// The message (already capped by the domain).
    pub message: String,
    /// How many times it happened.
    pub count: u32,
    /// When it was last seen.
    pub last_seen: Option<Timestamp>,
    /// Who reported it (`kubelet`).
    pub source: Option<Arc<str>>,
}

impl EventRow {
    /// The type as the tab says it (a word, so colour is not the only signal).
    pub fn type_label(&self) -> &'static str {
        match self.kind {
            EventType::Warning => "Warning",
            EventType::Normal => "Normal",
            EventType::Other => "Other",
        }
    }
}

impl From<&Event> for EventRow {
    fn from(event: &Event) -> Self {
        Self {
            kind: event.event_type,
            reason: event.reason.clone(),
            message: event.message.clone(),
            count: event.count,
            last_seen: event.last_seen.or(event.first_seen),
            source: event.reporting_component.clone(),
        }
    }
}

/// The rows of `events`, in their order (the feed gives them newest first, at most
/// [`MAX_EVENTS`]).
pub fn rows_of(events: &[Arc<Event>]) -> Vec<EventRow> {
    events
        .iter()
        .map(|event| EventRow::from(&**event))
        .collect()
}

#[cfg(test)]
mod tests;
