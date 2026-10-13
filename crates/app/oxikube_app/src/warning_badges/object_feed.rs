//! [`ObjectEventsFeed`]: the events about one object, for its Events tab.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::Stream;
use oxikube_domain::event::Event;
use oxikube_domain::ids::ClusterId;

use crate::events::{EventFilter, EventService, EventsState, EventsSubscription};

/// Most events the Events tab lists.
pub const MAX_OBJECT_EVENTS: usize = 200;

/// A live view of the events about one object, read from the service's index by the object's
/// uid: no watch, no API call. A stream that yields `()` when the events or the feed's state
/// changed (coalesced per poll), then [`events`](Self::events) and [`state`](Self::state) read
/// the new values. Dropping it drops its subscriptions.
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
        uid: Option<&str>,
        cluster_scoped_name: Option<&str>,
    ) -> Self {
        let mut uids: Vec<&str> = uid.into_iter().collect();
        if let Some(name) = cluster_scoped_name
            && !uids.contains(&name)
        {
            uids.push(name);
        }
        Self {
            subscriptions: uids
                .into_iter()
                .map(|uid| service.subscribe(cluster, EventFilter::new().object(uid)))
                .collect(),
        }
    }

    /// Whether the feed follows anything (the object had a uid to follow).
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
        if self.subscriptions.len() > 1 {
            events.sort_by_key(|e| std::cmp::Reverse(e.last_seen.or(e.first_seen)));
            events.truncate(MAX_OBJECT_EVENTS);
        }
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
