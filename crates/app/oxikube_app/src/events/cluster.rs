//! The service's shared state ([`Inner`]) and the life of one cluster's feed: started by the
//! first subscriber of a connected cluster, stopped after the idle grace once the last has left,
//! or at once when the cluster disconnects. One feed per cluster, whatever the subscribers.

use std::collections::HashMap;
use std::sync::Arc;
use std::task::Waker;

use futures::FutureExt as _;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{ClockPort, EventsMonitor, EventsWatchOptions};
use parking_lot::Mutex;

use super::buffer::EventBuffer;
use super::config::EventsConfig;
use super::driver;
use super::filter::EventFilter;
use super::state::EventsState;
use super::subscription::SubShared;
use crate::session::ClusterSessionManager;
use crate::store::{Spawner, TaskGuard, spawn_guarded};

/// One cluster's buffer, subscribers and feed.
pub(crate) struct ClusterEvents {
    pub(crate) buffer: EventBuffer,
    pub(crate) subscribers: HashMap<u64, Arc<SubShared>>,
    /// The feed task while one runs (abort-on-drop); a finished task after a final failure.
    pub(crate) feed: Option<TaskGuard>,
    /// Bumped when a feed starts: batches of an older feed are ignored.
    pub(crate) generation: u64,
    /// Bumped when the subscriber count changes: a grace timer of an older idle period is void.
    idle_token: u64,
    pub(crate) state: EventsState,
    pub(crate) monitor: Option<Arc<dyn EventsMonitor>>,
}

impl ClusterEvents {
    fn new(capacity: usize) -> Self {
        Self {
            buffer: EventBuffer::new(capacity),
            subscribers: HashMap::new(),
            feed: None,
            generation: 0,
            idle_token: 0,
            state: EventsState::Idle,
            monitor: None,
        }
    }

    /// Stops the feed and forgets the events; subscribers are told to read again.
    fn stop(&mut self, wakers: &mut Vec<Waker>) {
        self.feed = None;
        self.monitor = None;
        self.buffer.clear();
        self.set_state(EventsState::Idle, wakers);
        wakers.extend(self.subscribers.values().filter_map(|s| s.push_reset()));
    }

    pub(crate) fn set_state(&mut self, state: EventsState, wakers: &mut Vec<Waker>) {
        if self.state != state {
            self.state = state.clone();
            wakers.extend(
                self.subscribers
                    .values()
                    .filter_map(|s| s.push_state(state.clone())),
            );
        }
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) spawner: Option<Arc<dyn Spawner>>,
    pub(crate) follower: Option<TaskGuard>,
    pub(crate) config: EventsConfig,
    pub(crate) clusters: HashMap<ClusterId, ClusterEvents>,
    pub(crate) next_id: u64,
}

pub(crate) struct Inner {
    pub(crate) sessions: ClusterSessionManager,
    pub(crate) clock: Arc<dyn ClockPort>,
    pub(crate) state: Mutex<State>,
}

pub(crate) fn wake(wakers: Vec<Waker>) {
    for waker in wakers {
        waker.wake();
    }
}

impl Inner {
    /// Registers a subscriber of `cluster`, starting its feed if it is the first.
    pub(crate) fn subscribe(
        self: &Arc<Self>,
        cluster: &ClusterId,
        filter: EventFilter,
    ) -> (u64, Arc<SubShared>) {
        let shared = Arc::new(SubShared::new(filter));
        let id = {
            let mut state = self.state.lock();
            state.next_id += 1;
            let id = state.next_id;
            let capacity = state.config.buffer_for(cluster);
            let entry = state
                .clusters
                .entry(cluster.clone())
                .or_insert_with(|| ClusterEvents::new(capacity));
            entry.subscribers.insert(id, shared.clone());
            entry.idle_token += 1;
            id
        };
        self.reconcile(cluster);
        (id, shared)
    }

    /// Unregisters a subscriber; the last one starts the idle grace, after which the feed stops
    /// and the events are forgotten.
    pub(crate) fn unsubscribe(self: &Arc<Self>, cluster: &ClusterId, id: u64) {
        let (token, spawner, grace) = {
            let mut state = self.state.lock();
            let (spawner, grace) = (state.spawner.clone(), state.config.idle_grace);
            let Some(entry) = state.clusters.get_mut(cluster) else {
                return;
            };
            entry.subscribers.remove(&id);
            if !entry.subscribers.is_empty() {
                return;
            }
            entry.idle_token += 1;
            (entry.idle_token, spawner, grace)
        };
        match spawner {
            Some(spawner) if !grace.is_zero() => {
                // A timer, not Kubernetes work: it holds no guard and only ends an idle period
                // that is still current when it fires.
                let (inner, cluster, clock) =
                    (Arc::downgrade(self), cluster.clone(), self.clock.clone());
                spawner.spawn(
                    async move {
                        clock.sleep(grace).await;
                        if let Some(inner) = inner.upgrade() {
                            inner.expire(&cluster, token);
                        }
                    }
                    .boxed(),
                );
            }
            _ => self.expire(cluster, token),
        }
    }

    /// Ends `cluster`'s idle period `token`: its feed stops and its events are dropped.
    fn expire(&self, cluster: &ClusterId, token: u64) {
        let removed = {
            let mut state = self.state.lock();
            let idle = state
                .clusters
                .get(cluster)
                .is_some_and(|e| e.subscribers.is_empty() && e.idle_token == token);
            if !idle {
                return;
            }
            state.clusters.remove(cluster)
        };
        // Dropped outside the lock: the feed's guard aborts it, the buffer frees its events.
        drop(removed);
    }

    pub(crate) fn reconcile_all(self: &Arc<Self>) {
        let clusters: Vec<ClusterId> = self.state.lock().clusters.keys().cloned().collect();
        for cluster in clusters {
            self.reconcile(&cluster);
        }
    }

    /// Starts `cluster`'s feed when it has subscribers and is connected; stops it (and forgets
    /// its events) when it is not connected.
    pub(crate) fn reconcile(self: &Arc<Self>, cluster: &ClusterId) {
        let session = self
            .sessions
            .get(cluster)
            .filter(|session| session.is_connected());
        let port = session.as_ref().and_then(|session| session.events());
        let fallback: Vec<String> = session.as_ref().map_or_else(Vec::new, |session| {
            session
                .namespace_selection()
                .names()
                .map(str::to_owned)
                .collect()
        });
        let mut wakers = Vec::new();
        {
            let mut state = self.state.lock();
            let spawner = state.spawner.clone();
            let config = state.config.clone();
            let Some(entry) = state.clusters.get_mut(cluster) else {
                return;
            };
            match (port, spawner) {
                (Some(port), Some(spawner))
                    if entry.feed.is_none() && !entry.subscribers.is_empty() =>
                {
                    entry.generation += 1;
                    entry.set_state(EventsState::Warming, &mut wakers);
                    let capacity = config.buffer_for(cluster);
                    let options = EventsWatchOptions::default()
                        .capacity(capacity)
                        .fallback_namespaces(fallback);
                    let task = driver::run(driver::FeedTask {
                        inner: Arc::downgrade(self),
                        cluster: cluster.clone(),
                        generation: entry.generation,
                        port,
                        options,
                        clock: self.clock.clone(),
                        retry: (config.retry_initial, config.retry_max),
                    });
                    entry.feed = Some(spawn_guarded(&spawner, task));
                }
                (None, _) if entry.feed.is_some() || !entry.buffer.is_empty() => {
                    entry.stop(&mut wakers);
                }
                _ => {}
            }
        }
        wake(wakers);
    }

    /// Stops `cluster`'s feed and starts it again (a reconnect, a retry, a new bound).
    pub(crate) fn restart(self: &Arc<Self>, cluster: &ClusterId) {
        let old = self
            .state
            .lock()
            .clusters
            .get_mut(cluster)
            .and_then(|entry| entry.feed.take());
        drop(old);
        self.reconcile(cluster);
    }

    /// The namespace selection moved: a feed that is refused cluster-wide watches the selected
    /// namespaces instead, so one that failed or is missing scopes starts again.
    pub(crate) fn namespaces_changed(self: &Arc<Self>, cluster: &ClusterId) {
        let restart = self.state.lock().clusters.get(cluster).is_some_and(|e| {
            e.state.is_failed()
                || e.monitor
                    .as_ref()
                    .is_some_and(|m| !m.status().forbidden.is_empty())
        });
        if restart {
            self.restart(cluster);
        }
    }
}
