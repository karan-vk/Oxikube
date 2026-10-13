//! [`EventsSubscription`]: a consumer's handle on one cluster's events, and the mailbox
//! ([`SubShared`]) the service fills for it.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use futures::Stream;
use oxikube_domain::event::Event;
use oxikube_domain::ids::ClusterId;
use parking_lot::Mutex;

use super::changes::{Change, EventsChanged, Pending};
use super::cluster::Inner;
use super::filter::EventFilter;
use super::state::EventsState;

/// One subscriber's filter, pending changes and waker.
#[derive(Debug)]
pub(crate) struct SubShared(Mutex<SubState>);

#[derive(Debug)]
struct SubState {
    filter: EventFilter,
    pending: Pending,
    waker: Option<Waker>,
}

impl SubShared {
    /// A mailbox whose first item is a reset (so the subscriber reads what is held).
    pub(crate) fn new(filter: EventFilter) -> Self {
        Self(Mutex::new(SubState {
            filter,
            pending: Pending::reset(),
            waker: None,
        }))
    }

    /// Adds `changes`; returns the waker to wake (after the caller's locks are released) when
    /// something became pending.
    pub(crate) fn push(&self, changes: &[Change]) -> Option<Waker> {
        let mut state = self.0.lock();
        let SubState {
            filter, pending, ..
        } = &mut *state;
        pending.push(changes, filter);
        wake_if(&mut state)
    }

    /// Tells the subscriber to read everything again.
    pub(crate) fn push_reset(&self) -> Option<Waker> {
        let mut state = self.0.lock();
        state.pending.set_reset();
        wake_if(&mut state)
    }

    /// Tells the subscriber the feed's state changed.
    pub(crate) fn push_state(&self, feed: EventsState) -> Option<Waker> {
        let mut state = self.0.lock();
        state.pending.set_state(feed);
        wake_if(&mut state)
    }

    fn filter(&self) -> EventFilter {
        self.0.lock().filter.clone()
    }

    fn set_filter(&self, filter: EventFilter) -> Option<Waker> {
        let mut state = self.0.lock();
        if state.filter == filter {
            return None;
        }
        state.filter = filter;
        state.pending.set_reset();
        wake_if(&mut state)
    }

    fn poll(&self, cx: &mut Context<'_>) -> Poll<EventsChanged> {
        let mut state = self.0.lock();
        if state.pending.is_empty() {
            state.waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        Poll::Ready(state.pending.take())
    }
}

fn wake_if(state: &mut SubState) -> Option<Waker> {
    if state.pending.is_empty() {
        return None;
    }
    state.waker.take()
}

/// A live view of one cluster's events through an [`EventFilter`]: a [`Stream`] of
/// [`EventsChanged`], plus queries of what the service holds.
///
/// While at least one subscription of a cluster exists (and the cluster is connected) the
/// service runs that cluster's one events feed; when the last is dropped the feed stops after
/// the idle grace. The first item is a reset; later items coalesce everything that changed since
/// the last poll, so a view that polls once per frame gets at most one item per frame. The
/// stream does not end while the service lives.
pub struct EventsSubscription {
    pub(crate) inner: Arc<Inner>,
    pub(crate) cluster: ClusterId,
    pub(crate) id: u64,
    pub(crate) shared: Arc<SubShared>,
}

impl std::fmt::Debug for EventsSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventsSubscription")
            .field("cluster", &self.cluster)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl EventsSubscription {
    /// The cluster.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// The filter changes are reported through.
    pub fn filter(&self) -> EventFilter {
        self.shared.filter()
    }

    /// Replaces the filter; the next item is a reset. No feed restarts.
    pub fn set_filter(&self, filter: EventFilter) {
        if let Some(waker) = self.shared.set_filter(filter) {
            waker.wake();
        }
    }

    /// Every held event that matches the filter, newest first.
    pub fn query(&self) -> Vec<Arc<Event>> {
        self.latest(usize::MAX)
    }

    /// The newest `n` held events that match the filter.
    pub fn latest(&self, n: usize) -> Vec<Arc<Event>> {
        let filter = self.filter();
        self.inner.latest(&self.cluster, &filter, n)
    }

    /// The state of the cluster's feed now.
    pub fn state(&self) -> EventsState {
        self.inner.state_of(&self.cluster)
    }
}

impl Stream for EventsSubscription {
    type Item = EventsChanged;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<EventsChanged>> {
        self.shared.poll(cx).map(Some)
    }
}

impl Drop for EventsSubscription {
    fn drop(&mut self) {
        self.inner.unsubscribe(&self.cluster, self.id);
    }
}
