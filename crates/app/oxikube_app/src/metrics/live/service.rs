//! [`MetricsService`]: who looks at which cluster's metrics, and the poller each such cluster
//! gets. The loop itself is in `poller`.

use std::collections::HashMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::{FutureExt as _, StreamExt as _};
use oxikube_domain::ids::ClusterId;
use oxikube_ports::ClockPort;
use parking_lot::Mutex;
use tokio::sync::broadcast;

use super::cadence::Cadence;
use super::config::{LiveMetricsConfig, LiveMetricsConfigTable};
use super::handles::{MetricsInterest, MetricsVisibility};
use super::reconcile::follow;
use super::state::{MetricsKind, MetricsState};
use super::usage::ClusterUsage;
use crate::session::ClusterSessionManager;
use crate::store::{Spawner, TaskGuard, spawn_guarded};

/// Capacity of the change broadcast; a subscriber that falls behind re-reads the state.
const CHANGE_CAPACITY: usize = 64;

/// One cluster's consumers, cache and poller.
pub(super) struct Entry {
    pub(super) usage: Arc<ClusterUsage>,
    pub(super) config: LiveMetricsConfig,
    /// Consumers per [`MetricsKind`].
    pub(super) interest: [usize; 2],
    /// Windows showing the cluster's tab.
    pub(super) visible: usize,
    pub(super) cadence: [Cadence; 2],
    /// The poller while the cluster is watched; dropping it aborts the loop.
    pub(super) poller: Option<TaskGuard>,
}

impl Entry {
    fn new(config: LiveMetricsConfig) -> Self {
        let usage = Arc::new(ClusterUsage::new());
        usage.set_thresholds(config.thresholds);
        Self {
            usage,
            config,
            interest: [0; 2],
            visible: 0,
            cadence: Default::default(),
            poller: None,
        }
    }

    /// The kinds someone is looking at.
    pub(super) fn wanted(&self) -> impl Iterator<Item = MetricsKind> + '_ {
        MetricsKind::ALL
            .into_iter()
            .filter(|kind| self.interest[kind.index()] > 0)
    }
}

#[derive(Default)]
pub(super) struct State {
    pub(super) spawner: Option<Arc<dyn Spawner>>,
    follower: Option<TaskGuard>,
    config: LiveMetricsConfigTable,
    pub(super) clusters: HashMap<ClusterId, Entry>,
}

impl State {
    /// `cluster`'s entry, made with its configured settings on first sight.
    pub(super) fn entry(&mut self, cluster: &ClusterId) -> &mut Entry {
        let config = self.config.get(cluster);
        self.clusters
            .entry(cluster.clone())
            .or_insert_with(|| Entry::new(config))
    }
}

pub(super) struct Inner {
    pub(super) sessions: ClusterSessionManager,
    pub(super) clock: Arc<dyn ClockPort>,
    pub(super) state: Mutex<State>,
    pub(super) changes: broadcast::Sender<ClusterId>,
}

/// Live CPU and memory from metrics-server, polled per cluster while someone looks (E13-S03).
/// See the [module docs](super).
#[derive(Clone)]
pub struct MetricsService {
    pub(super) inner: Arc<Inner>,
}

impl std::fmt::Debug for MetricsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsService")
            .field("clusters", &self.inner.state.lock().clusters.len())
            .finish_non_exhaustive()
    }
}

impl MetricsService {
    /// A service over `sessions`, timing its polls on `clock`. Nothing polls until
    /// [`attach`](Self::attach) (or [`start`](Self::start)) gives it a spawner.
    pub fn new(sessions: ClusterSessionManager, clock: Arc<dyn ClockPort>) -> Self {
        let (changes, _) = broadcast::channel(CHANGE_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                sessions,
                clock,
                state: Mutex::new(State::default()),
                changes,
            }),
        }
    }

    /// Lets the pollers run on `spawner` (the Tokio bridge). The first spawner stays.
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

    /// The loop that follows the session updates: a cluster that disconnects stops polling and
    /// forgets its samples, one that connects polls if it is watched, a new namespace selection
    /// polls the pods again. It only waits, so a host runs it on its foreground executor.
    pub fn follow(&self) -> BoxFuture<'static, ()> {
        let updates = self.inner.sessions.subscribe();
        follow(Arc::downgrade(&self.inner), updates).boxed()
    }

    /// [`follow`](Self::follow) and [`attach`](Self::attach) on `spawner`, for a host without an
    /// executor of its own (tests). The follower is aborted with the service.
    pub fn start(&self, spawner: Arc<dyn Spawner>) {
        if self.inner.state.lock().follower.is_some() {
            return;
        }
        let follower = self.follow();
        self.attach(spawner.clone());
        self.inner.state.lock().follower = Some(spawn_guarded(&spawner, follower));
    }

    /// Replaces every cluster's interval and thresholds (the `metrics` setting changed). A
    /// running poller whose interval changed starts again on the new one.
    pub fn set_config(&self, table: LiveMetricsConfigTable) {
        let restart: Vec<ClusterId> = {
            let mut state = self.inner.state.lock();
            let mut restart = Vec::new();
            for (cluster, entry) in &mut state.clusters {
                let config = table.get(cluster);
                entry.usage.set_thresholds(config.thresholds);
                if config.poll_interval != entry.config.poll_interval && entry.poller.is_some() {
                    entry.poller = None;
                    restart.push(cluster.clone());
                }
                entry.config = config;
            }
            state.config = table;
            restart
        };
        for cluster in restart {
            self.inner.reconcile(&cluster);
        }
    }

    /// `cluster`'s samples: the source its Pods and Nodes columns read. The same `Arc` for the
    /// life of the service.
    pub fn usage(&self, cluster: &ClusterId) -> Arc<ClusterUsage> {
        self.inner.state.lock().entry(cluster).usage.clone()
    }

    /// How the last poll of `kind` in `cluster` went.
    pub fn state(&self, cluster: &ClusterId, kind: MetricsKind) -> MetricsState {
        self.inner
            .state
            .lock()
            .clusters
            .get(cluster)
            .map(|entry| entry.usage.state(kind))
            .unwrap_or_default()
    }

    /// Registers a consumer of `kind` in `cluster` (a table on screen). The cluster is polled
    /// while it has a consumer and its tab is shown; dropping the handle unregisters it.
    pub fn watch(&self, cluster: &ClusterId, kind: MetricsKind) -> MetricsInterest {
        let first = {
            let mut state = self.inner.state.lock();
            let entry = state.entry(cluster);
            entry.interest[kind.index()] += 1;
            let first = entry.interest[kind.index()] == 1;
            if first {
                entry.usage.set_loading(kind);
                // A kind nobody watched joins at once: the running poller is restarted, and
                // polls it now.
                entry.poller = None;
            }
            first
        };
        if first {
            self.inner.reconcile(cluster);
        }
        MetricsInterest::new(Arc::downgrade(&self.inner), cluster.clone(), kind)
    }

    /// A handle a window moves to the cluster tab it shows ([`MetricsVisibility::show`]): a
    /// cluster no window shows is not polled. Dropping it shows nothing.
    pub fn visibility(&self) -> MetricsVisibility {
        MetricsVisibility::new(Arc::downgrade(&self.inner))
    }

    /// Whether `cluster` is being polled now (it is watched, shown and connected).
    pub fn is_polling(&self, cluster: &ClusterId) -> bool {
        self.inner
            .state
            .lock()
            .clusters
            .get(cluster)
            .is_some_and(|entry| entry.poller.is_some())
    }

    /// A stream that yields whenever `cluster`'s samples or states change (once per poll), and
    /// after a lag. Ends with the service.
    pub fn changes(&self, cluster: &ClusterId) -> BoxStream<'static, ()> {
        let cluster = cluster.clone();
        let rx = self.inner.changes.subscribe();
        futures::stream::unfold(rx, move |mut rx| {
            let cluster = cluster.clone();
            async move {
                loop {
                    match rx.recv().await {
                        Ok(changed) if changed == cluster => return Some(((), rx)),
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(_)) => return Some(((), rx)),
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                }
            }
        })
        .boxed()
    }
}
