//! [`ObjectEventsFeed`]: the events about one object, for its Events tab.

use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::Stream;
use oxikube_domain::event::Event;
use oxikube_domain::ids::ClusterId;

use crate::events::{
    EventFilter, EventKey, EventService, EventsState, EventsSubscription, ObjectTarget, seen_at,
};

/// Most events the Events tab lists.
pub const MAX_OBJECT_EVENTS: usize = 200;

/// A live view of the events about one object, read from the service's index by the object's
/// uid and by its name: no watch, no API call. A stream that yields `()` when the events or the
/// feed's state changed (coalesced per poll), then [`events`](Self::events) and [`state`](Self::state) read
/// the new values. Dropping it drops its subscriptions.
///
/// The events whose writer gave no uid are found by the object's name (kind, namespace and
/// name), so they show in the tab too.
///
/// A cluster-scoped object (a Node) is also followed under its name: the kubelet records a
/// Node's events with the node's name as the uid.
pub struct ObjectEventsFeed {
    subscriptions: Vec<EventsSubscription>,
}

impl std::fmt::Debug for ObjectEventsFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectEventsFeed")
            .field("subscriptions", &self.subscriptions.len())
            .finish()
    }
}

impl ObjectEventsFeed {
    pub(super) fn new(
        service: &EventService,
        cluster: &ClusterId,
        target: &ObjectTarget,
        cluster_scoped_name: Option<&str>,
    ) -> Self {
        let identity = target.identity().cloned();
        // The object's uid, and (for a cluster-scoped object) its name as the kubelet writes it.
        let mut uids: Vec<Arc<str>> = Vec::new();
        if let Some(uid) = target.uid_value() {
            uids.push(Arc::from(uid));
        }
        if let Some(name) = cluster_scoped_name
            && !uids.iter().any(|uid| &**uid == name)
        {
            uids.push(Arc::from(name));
        }
        let targets: Vec<ObjectTarget> = match (&identity, uids.is_empty()) {
            // No uid to follow: the name alone finds the events that gave none.
            (Some(identity), true) => vec![ObjectTarget::named(None, identity.clone())],
            (Some(identity), false) => uids
                .iter()
                .map(|uid| ObjectTarget::named(Some(uid.clone()), identity.clone()))
                .collect(),
            (None, _) => uids
                .iter()
                .map(|uid| ObjectTarget::uid(uid.clone()))
                .collect(),
        };
        Self {
            subscriptions: targets
                .into_iter()
                .map(|target| service.subscribe(cluster, EventFilter::new().object_target(target)))
                .collect(),
        }
    }

    /// Whether the feed follows anything (the object had a uid or a name to follow).
    pub fn is_empty(&self) -> bool {
        self.subscriptions.is_empty()
    }

    /// The events about the object, newest (by last seen) first, at most
    /// [`MAX_OBJECT_EVENTS`].
    pub fn events(&self) -> Vec<Arc<Event>> {
        let mut events: Vec<Arc<Event>> = self
            .subscriptions
            .iter()
            .flat_map(|s| s.latest(MAX_OBJECT_EVENTS))
            .collect();
        events.sort_by_key(|e| std::cmp::Reverse(seen_at(e)));
        // Two subscriptions (a cluster-scoped object's uid and its name) hold the same event
        // when the event gave no uid: it is listed once.
        let mut seen: HashSet<EventKey> = HashSet::with_capacity(events.len());
        events.retain(|event| seen.insert(EventKey::of(event)));
        events.truncate(MAX_OBJECT_EVENTS);
        events
    }

    /// The state of the cluster's events feed.
    pub fn state(&self) -> EventsState {
        self.subscriptions
            .first()
            .map_or(EventsState::Idle, EventsSubscription::state)
    }
}

impl Stream for ObjectEventsFeed {
    type Item = ();

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<()>> {
        // Every subscription is polled so each registers the waker; any item is one change.
        let mut ready = false;
        for subscription in &mut self.subscriptions {
            while Pin::new(&mut *subscription).poll_next(cx).is_ready() {
                ready = true;
            }
        }
        if ready {
            Poll::Ready(Some(()))
        } else {
            Poll::Pending
        }
    }
}
