//! The Events tab's feed: the object's events from the app's `EventService` index.
//!
//! It starts the first time the tab is shown (the Overview alone costs nothing) and needs the
//! object's uid, so it waits for the object when the tab is shown first. It is a read of the
//! service's one cluster-wide feed (`ObjectEventsFeed`): no watch, no API call. The service
//! follows a reconnect by itself, so the view has nothing to re-subscribe.

use std::pin::Pin;
use std::task::Poll;

use futures::Stream as _;
use gpui::Context;
use oxikube_app::EventsState;
use oxikube_runtime::notify_coalesced;

use super::events::rows_of;
use super::follow::Feed;
use super::view::DetailView;
use crate::warnings::ObjectEvents;

impl DetailView {
    /// Starts the Events feed, once.
    pub(super) fn start_events(&mut self, cx: &mut Context<Self>) {
        if self.events.started {
            return;
        }
        self.events.started = true;
        self.subscribe_events(cx);
    }

    /// A version of the object arrived: a tab shown before the object was known can follow its
    /// events now, and a tab following an object that was deleted and recreated under the same
    /// name (a new uid) follows the new one.
    pub(super) fn object_known_for_events(&mut self, cx: &mut Context<Self>) {
        if !self.events.started {
            return;
        }
        let uid = self.object.as_ref().and_then(|o| o.meta().uid.clone());
        if self.events.feed.is_none() || self.events.uid != uid {
            self.subscribe_events(cx);
        }
    }

    fn subscribe_events(&mut self, cx: &mut Context<Self>) {
        let Some(events) = ObjectEvents::get(cx) else {
            self.events.error = Some("the events service is not running".to_owned());
            cx.notify();
            return;
        };
        // Without the object there is no uid to follow: the skeleton shows until it arrives.
        let Some(object) = self.object.as_ref() else {
            return;
        };
        let meta = object.meta();
        // A Node's events are recorded under its name as well as its uid.
        let cluster_scoped_name = self.target.namespace().is_none().then_some(&*meta.name);
        let feed = events.object_feed(
            &self.target.cluster,
            meta.uid.as_deref(),
            cluster_scoped_name,
        );
        self.events.feed = Some(feed);
        self.events.uid = meta.uid.clone();
        self.events.error = None;
        self.refresh_events(cx);
        self.events.task = Some(Self::poll(Feed::Events, cx));
    }

    /// Polls the feed for changes; whether to keep polling.
    pub(super) fn drain_events(
        &mut self,
        task_cx: &mut std::task::Context<'_>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(feed) = self.events.feed.as_mut() else {
            return false;
        };
        let mut changed = false;
        loop {
            match Pin::new(&mut *feed).poll_next(task_cx) {
                Poll::Ready(Some(())) => changed = true,
                Poll::Ready(None) => return false,
                Poll::Pending => break,
            }
        }
        if changed {
            self.refresh_events(cx);
            notify_coalesced(cx);
        }
        true
    }

    /// Reads the object's events and the feed's state into the tab.
    fn refresh_events(&mut self, cx: &mut Context<Self>) {
        let Some(feed) = self.events.feed.as_ref() else {
            return;
        };
        let rows = rows_of(&feed.events());
        let state = feed.state();
        self.events.error = match &state {
            EventsState::Failed { message, .. } => Some(message.clone()),
            _ => None,
        };
        // Events the service holds already show at once; the skeleton is for a feed that has
        // listed nothing yet.
        self.events.ready = feed.is_empty()
            || !rows.is_empty()
            || matches!(state, EventsState::Ready | EventsState::Retrying { .. });
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
