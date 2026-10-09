//! The Events tab's data: the events of the object, from the store's `Event` feed.
//!
//! The feed is the namespace's events (the store shares it with every other view of the same
//! scope); this keeps the ones that are about the object, newest first. It starts when the tab is
//! first shown, so opening a drawer on the Overview costs no extra watch.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_app::store::StoreObject;
use oxikube_domain::event::{Event, EventType};
use oxikube_domain::ids::{ClusterId, ResourceRef};

/// Most events kept.
pub const MAX_EVENTS: usize = 200;

/// One event as the tab lists it.
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

/// The events of `objects` (an `Event` feed's rows) that are about `target`, newest first, at most
/// [`MAX_EVENTS`]. `uid` is the object's own, to tell it from an earlier object of the same name.
pub fn events_about(
    cluster: &ClusterId,
    target: &ResourceRef,
    uid: Option<&str>,
    objects: &[Arc<StoreObject>],
) -> Vec<EventRow> {
    let mut rows: Vec<EventRow> = objects
        .iter()
        .filter_map(|object| about(cluster, target, uid, object))
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.last_seen));
    rows.truncate(MAX_EVENTS);
    rows
}

fn about(
    cluster: &ClusterId,
    target: &ResourceRef,
    uid: Option<&str>,
    object: &StoreObject,
) -> Option<EventRow> {
    let resource = object.resource()?;
    let json = resource.json();
    // Cheap pre-check on the raw JSON: most events in the namespace are about other objects.
    let named = json
        .pointer("/involvedObject/name")
        .or_else(|| json.pointer("/regarding/name"))
        .and_then(|name| name.as_str())?;
    if named != &*target.name {
        return None;
    }
    // Decoded only for the events that name the target: most events in the namespace do not.
    let event = Event::from_json(cluster, &json.to_value()).ok()?;
    let same_object = event.regarding.gvk.kind == target.gvk.kind
        && event.regarding.namespace() == target.namespace();
    let same_uid = match (uid, event.regarding_uid.as_deref()) {
        // The kubelet and kube-proxy record a Node's events with the node's name as the uid
        // (`kubectl describe node` looks them up the same way), so a cluster-scoped object
        // also matches on its name.
        (Some(ours), Some(theirs)) => {
            ours == theirs || (target.namespace().is_none() && theirs == &*target.name)
        }
        _ => true,
    };
    (same_object && same_uid).then(|| EventRow::from(&event))
}

#[cfg(test)]
mod tests;
