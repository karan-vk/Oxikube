//! The Events tab's feed: the namespace's `Event` objects from the store, filtered to the ones
//! about this object.
//!
//! It starts the first time the tab is shown (the Overview alone costs no watch). The query
//! shares the feed any other view of the namespace's events has; the object's events are picked
//! out of the delta as it arrives, off render. An object that is cluster-scoped has its events in
//! the `default` namespace.

use gpui::Context;
use oxikube_app::store::{FeedState, StoreDelta, StoreQuery};
use oxikube_domain::ids::Gvk;

use super::events::events_about;
use super::follow::Feed;
use super::view::DetailView;

/// Where the events about a cluster-scoped object are recorded.
const CLUSTER_SCOPED_EVENTS_NAMESPACE: &str = "default";

impl DetailView {
    /// Starts the Events feed, once.
    pub(super) fn start_events(&mut self, cx: &mut Context<Self>) {
        if self.events.started {
            return;
        }
        self.events.started = true;
        self.subscribe_events(cx);
    }

    /// Re-subscribes the Events feed after a reconnect (a new store), when it was started.
    pub(super) fn restart_events(&mut self, cx: &mut Context<Self>) {
        if !self.events.started {
            return;
        }
        let same = self
            .store
            .as_ref()
            .zip(self.events_store.as_ref())
            .is_some_and(|(a, b)| a.is_same(b));
        if !same || self.events.subscription.is_none() {
            self.subscribe_events(cx);
        }
    }

    fn subscribe_events(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.connected_store() else {
            return;
        };
        let namespace = self
            .target
            .namespace()
            .unwrap_or(CLUSTER_SCOPED_EVENTS_NAMESPACE);
        let events = Gvk::new("", "v1", "Event");
        let scope = self.scope_for(&store, &events, Some(namespace));
        let query = StoreQuery::new(events, scope);
        self.events.subscription = Some(store.subscribe(query));
        self.events_store = Some(store);
        self.events.feed.clear();
        self.events.rows.clear();
        self.events.ready = false;
        self.events.error = None;
        self.events.task = Some(Self::poll(Feed::Events, cx));
    }

    /// Applies one delta of the events feed.
    pub(super) fn apply_events(&mut self, delta: StoreDelta, cx: &mut Context<Self>) {
        delta.apply_to(&mut self.events.feed);
        match &delta.state {
            FeedState::Ready => {
                self.events.ready = true;
                self.events.error = None;
            }
            FeedState::Forbidden { message }
            | FeedState::Unauthorized { message }
            | FeedState::Failed { message, .. } => {
                self.events.error = Some(message.clone());
            }
            FeedState::Warming | FeedState::Retrying { .. } => {}
        }
        let uid = self
            .object
            .as_ref()
            .and_then(|object| object.meta().uid.clone());
        let rows = events_about(
            &self.target.cluster,
            &self.target,
            uid.as_deref(),
            &self.events.feed,
        );
        // New events arrive at the top: splicing at the front keeps the scroll where it is.
        let old = self.events_list.item_count();
        match rows.len().cmp(&old) {
            std::cmp::Ordering::Greater => self.events_list.splice(0..0, rows.len() - old),
            std::cmp::Ordering::Less => self.events_list.splice(0..old - rows.len(), 0),
            std::cmp::Ordering::Equal => {}
        }
        if !rows.is_empty() {
            self.events_list.remeasure_items(0..rows.len());
        }
        self.events.rows = rows;
        cx.notify();
    }
}
