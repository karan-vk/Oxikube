//! [`EventFilter`]: which events a query or a subscription wants.
//!
//! Every part is optional and the parts combine with AND; inside a part the values combine with
//! OR (`reasons(["BackOff", "Failed"])` is either reason). The text pattern is compiled once,
//! when the filter is built, and never per event: it is the `/` filter's [`TextPattern`] (a
//! case-insensitive substring, or a regular expression when the text has regex syntax).
//!
//! The object part is an [`ObjectTarget`]: the uid of the object the events are about, and the
//! name (kind, namespace, name) the events that gave no uid are matched on.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::event::{Event, EventType};

use super::object::{ObjectIdentity, ObjectTarget};
use crate::store::TextPattern;

/// A text pattern that did not compile.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid event filter pattern {pattern:?}: {message}")]
pub struct EventFilterError {
    /// The text as typed.
    pub pattern: String,
    /// The regex engine's message, on one line.
    pub message: String,
}

/// Which events to return, see the module docs. The default matches every event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventFilter {
    types: Vec<EventType>,
    reasons: Vec<Arc<str>>,
    kinds: Vec<Arc<str>>,
    namespaces: Vec<Arc<str>>,
    object: Option<ObjectTarget>,
    since: Option<Timestamp>,
    text: Option<TextPattern>,
}

impl EventFilter {
    /// A filter that matches every event.
    pub fn new() -> Self {
        Self::default()
    }

    /// Only `Warning` events (the events view's "faults only", the warnings panel).
    pub fn warnings() -> Self {
        Self::new().types([EventType::Warning])
    }

    /// Only events of these types (none: any type).
    #[must_use]
    pub fn types(mut self, types: impl IntoIterator<Item = EventType>) -> Self {
        self.types.clear();
        for t in types {
            if !self.types.contains(&t) {
                self.types.push(t);
            }
        }
        self
    }

    /// Only events with one of these reasons, compared exactly (`BackOff`); none: any reason.
    #[must_use]
    pub fn reasons<S: Into<Arc<str>>>(mut self, reasons: impl IntoIterator<Item = S>) -> Self {
        self.reasons = sorted(reasons);
        self
    }

    /// Only events about objects of these kinds (`Pod`; case-insensitive); none: any kind.
    #[must_use]
    pub fn kinds<S: Into<Arc<str>>>(mut self, kinds: impl IntoIterator<Item = S>) -> Self {
        self.kinds = sorted(kinds);
        self
    }

    /// Only events stored in these namespaces (the event's own `namespace`, as `kubectl get
    /// events -n` filters); none: every namespace.
    #[must_use]
    pub fn namespaces<S: Into<Arc<str>>>(
        mut self,
        namespaces: impl IntoIterator<Item = S>,
    ) -> Self {
        self.namespaces = sorted(namespaces);
        self
    }

    /// Only events about the object with this uid (the involved object, `regarding.uid`).
    #[must_use]
    pub fn object(mut self, uid: impl Into<Arc<str>>) -> Self {
        self.object = Some(ObjectTarget::uid(uid));
        self
    }

    /// Only events about this object: the events that carry its uid, and the events that gave
    /// no uid and name it (see [`ObjectTarget`]).
    #[must_use]
    pub fn object_target(mut self, target: ObjectTarget) -> Self {
        self.object = Some(target);
        self
    }

    /// Only events last seen at or after `since`.
    #[must_use]
    pub fn since(mut self, since: Timestamp) -> Self {
        self.since = Some(since);
        self
    }

    /// Only events whose reason, message, object kind or name, or reporting component contains
    /// `text` (case-insensitive), or matches it as a regular expression when it has regex syntax.
    /// An empty `text` removes the text part.
    ///
    /// # Errors
    ///
    /// [`EventFilterError`] when `text` is a regular expression that does not compile.
    pub fn text(mut self, text: &str) -> Result<Self, EventFilterError> {
        if text.is_empty() {
            self.text = None;
            return Ok(self);
        }
        let pattern = TextPattern::compile(text).map_err(|message| EventFilterError {
            pattern: text.to_owned(),
            message,
        })?;
        self.text = Some(pattern);
        Ok(self)
    }

    /// Whether this filter matches every event.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The involved-object uid this filter is limited to.
    pub fn object_uid(&self) -> Option<&str> {
        self.object.as_ref().and_then(ObjectTarget::uid_value)
    }

    /// Whether the filter is limited to one object, even one it knows nothing about.
    pub fn has_object(&self) -> bool {
        self.object.is_some()
    }

    /// The name of the object this filter is limited to, when it knows one: the events that
    /// gave no uid are matched on it.
    pub fn object_identity(&self) -> Option<&ObjectIdentity> {
        self.object.as_ref().and_then(ObjectTarget::identity)
    }

    /// The earliest last-seen time this filter accepts.
    pub fn since_time(&self) -> Option<Timestamp> {
        self.since
    }

    /// Whether only `Warning` events match.
    pub fn is_warnings_only(&self) -> bool {
        self.types == [EventType::Warning]
    }

    /// The namespace this filter is limited to, when it names exactly one.
    pub(super) fn single_namespace(&self) -> Option<&str> {
        match self.namespaces.as_slice() {
            [one] => Some(one),
            _ => None,
        }
    }

    /// Whether `event` matches every part.
    pub fn matches(&self, event: &Event) -> bool {
        (self.types.is_empty() || self.types.contains(&event.event_type))
            && (self.reasons.is_empty() || contains(&self.reasons, &event.reason))
            && (self.kinds.is_empty()
                || self
                    .kinds
                    .iter()
                    .any(|k| k.eq_ignore_ascii_case(&event.regarding.gvk.kind)))
            && (self.namespaces.is_empty()
                || event
                    .namespace
                    .as_deref()
                    .is_some_and(|ns| contains(&self.namespaces, ns)))
            && self
                .object
                .as_ref()
                .is_none_or(|target| target.matches(event))
            && self
                .since
                .is_none_or(|since| super::key::seen_at(event) >= since)
            && self
                .text
                .as_ref()
                .is_none_or(|text| text_matches(text, event))
    }
}

fn sorted<S: Into<Arc<str>>>(values: impl IntoIterator<Item = S>) -> Vec<Arc<str>> {
    let mut values: Vec<Arc<str>> = values.into_iter().map(Into::into).collect();
    values.sort();
    values.dedup();
    values
}

fn contains(sorted: &[Arc<str>], value: &str) -> bool {
    sorted.binary_search_by(|v| (**v).cmp(value)).is_ok()
}

fn text_matches(text: &TextPattern, event: &Event) -> bool {
    text.matches(&event.reason)
        || text.matches(&event.message)
        || text.matches(&event.regarding.name)
        || text.matches(&event.regarding.gvk.kind)
        || event
            .reporting_component
            .as_deref()
            .is_some_and(|c| text.matches(c))
}
