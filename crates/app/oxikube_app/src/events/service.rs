//! [`EventService`]: the app's one owner of every cluster's events.

use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use oxikube_domain::event::Event;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{ClockPort, EventsStatus};
use parking_lot::Mutex;

use super::buffer::EventBuffer;
use super::cluster::{Inner, State};
use super::config::EventsConfig;
use super::filter::EventFilter;
use super::follow::follow;
use super::state::EventsState;
use super::subscription::EventsSubscription;
use crate::session::ClusterSessionManager;
use crate::store::{Spawner, spawn_guarded};

/// Every cluster's events, one feed per cluster, in a bounded, de-duplicated and indexed
/// buffer. See the [module docs](super).
///
/// Cheap to clone: a handle on shared state. Every method returns at once; the feeds run on the
/// spawner given to [`attach`](Self::attach), and queries read memory under a short lock.
#[derive(Clone)]
pub struct EventService {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for EventService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventService")
            .field("clusters", &self.inner.state.lock().clusters.len())
            .finish_non_exhaustive()
    }
}

impl EventService {
    /// A service over `sessions`, pausing between retries (and timing the idle grace) on
    /// `clock`. Nothing runs until [`attach`](Self::attach) (or [`start`](Self::start)) gives it
    /// a spawner.
    pub fn new(
        sessions: ClusterSessionManager,
        clock: Arc<dyn ClockPort>,
        config: EventsConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                sessions,
                clock,
                state: Mutex::new(State {
                    config,
                    ..State::default()
                }),
            }),
        }
    }

    /// Lets the feeds run on `spawner` (the Tokio bridge). The first spawner stays.
    pub fn attach(&self, spawner: Arc<dyn Spawner>) {
        {
            let mut state = self.inner.state.lock();
            if state.spawner.is_some() {
                return;
            }
            state.spawner = Some(spawner);
        }
        self.inner.reconcile_all();
    }

    /// The loop that follows the session updates (see `follow`). It only waits, so a host runs
    /// it on its foreground executor.
    pub fn follow(&self) -> BoxFuture<'static, ()> {
        let updates = self.inner.sessions.subscribe();
        follow(Arc::downgrade(&self.inner), updates).boxed()
    }

    /// [`follow`](Self::follow) and [`attach`](Self::attach) on `spawner`, for a host without an
    /// executor of its own (tests, headless runs). The follower is aborted with the service.
    pub fn start(&self, spawner: Arc<dyn Spawner>) {
        if self.inner.state.lock().follower.is_some() {
            return;
        }
        let follower = self.follow();
        self.attach(spawner.clone());
        self.inner.state.lock().follower = Some(spawn_guarded(&spawner, follower));
    }

    /// Subscribes to `cluster`'s events through `filter`. The first subscriber of a connected
    /// cluster starts its feed; the feed runs while any subscription lives (and the idle grace
    /// after the last is dropped).
    pub fn subscribe(&self, cluster: &ClusterId, filter: EventFilter) -> EventsSubscription {
        let (id, shared) = self.inner.subscribe(cluster, filter);
        EventsSubscription {
            inner: self.inner.clone(),
            cluster: cluster.clone(),
            id,
            shared,
        }
    }

    /// Every event of `cluster` that matches `filter`, newest (by last seen) first.
    pub fn query(&self, cluster: &ClusterId, filter: &EventFilter) -> Vec<Arc<Event>> {
        self.latest(cluster, filter, usize::MAX)
    }

    /// The newest `n` events of `cluster` that match `filter`.
    pub fn latest(&self, cluster: &ClusterId, filter: &EventFilter, n: usize) -> Vec<Arc<Event>> {
        self.inner.latest(cluster, filter, n)
    }

    /// Runs `read` on `cluster`'s buffer under the service's lock (`None` when the cluster has
    /// none): for a caller that walks [`EventBuffer::query`] lazily or reads the `O(1)`
    /// [`EventBuffer::object_counts`] of many rows at once. Keep `read` short.
    pub fn read<R>(&self, cluster: &ClusterId, read: impl FnOnce(&EventBuffer) -> R) -> Option<R> {
        self.inner.read(cluster, read)
    }

    /// The state of `cluster`'s feed.
    pub fn state(&self, cluster: &ClusterId) -> EventsState {
        self.inner.state_of(cluster)
    }

    /// What the adapter reports about `cluster`'s running feed: the API watched, its counters,
    /// the scopes the user may not read. `None` while no feed runs.
    pub fn status(&self, cluster: &ClusterId) -> Option<EventsStatus> {
        self.inner.status_of(cluster)
    }

    /// Whether `cluster`'s feed runs: warming, ready or retrying.
    pub fn is_watching(&self, cluster: &ClusterId) -> bool {
        matches!(
            self.state(cluster),
            EventsState::Warming | EventsState::Ready | EventsState::Retrying { .. }
        )
    }

    /// Opens `cluster`'s feed again (after a final failure, the view's "Retry").
    pub fn retry(&self, cluster: &ClusterId) {
        self.inner.restart(cluster);
    }

    /// Replaces the bounds and timings (the `events.buffer` setting changed): see
    /// [`EventBuffer::set_capacity`] for what a new bound does to the events held.
    pub fn set_config(&self, config: EventsConfig) {
        self.inner.set_config(config);
    }

    /// The config in effect.
    pub fn config(&self) -> EventsConfig {
        self.inner.state.lock().config.clone()
    }
}
