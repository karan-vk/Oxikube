//! [`MetricsService`]: who looks at which cluster's metrics, and the poller each such cluster
//! gets. The loop itself is in `poller`.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::{FutureExt as _, StreamExt as _};
use oxikube_domain::ids::ClusterId;
use oxikube_ports::ClockPort;
use parking_lot::Mutex;
use tokio::sync::broadcast;

use super::cadence::Cadence;
use super::config::{LiveMetricsConfig, LiveMetricsConfigTable};
use super::poller::poll_loop;
use super::state::{MetricsKind, MetricsState};
use super::usage::ClusterUsage;
use crate::session::{ClusterSessionManager, SessionChange, SessionUpdates};
use crate::store::{Spawner, TaskGuard, spawn_guarded};

/// Capacity of the change broadcast; a subscriber that falls behind re-reads the state.
const CHANGE_CAPACITY: usize = 64;

/// One cluster's consumers, cache and poller.
pub(super) struct Entry {
    pub(super) usage: Arc<ClusterUsage>,
    pub(super) config: LiveMetricsConfig,
    /// Consumers per [`MetricsKind`].
    interest: [usize; 2],
    /// Windows showing the cluster's tab.
    visible: usize,
    pub(super) cadence: [Cadence; 2],
    /// The poller while the cluster is watched; dropping it aborts the loop.
    poller: Option<TaskGuard>,
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
    spawner: Option<Arc<dyn Spawner>>,
    follower: Option<TaskGuard>,
    config: LiveMetricsConfigTable,
    pub(super) clusters: HashMap<ClusterId, Entry>,
}

pub(super) struct Inner {
    pub(super) sessions: ClusterSessionManager,
    pub(super) clock: Arc<dyn ClockPort>,
    pub(super) state: Mutex<State>,
    changes: broadcast::Sender<ClusterId>,
}

/// Live CPU and memory from metrics-server, polled per cluster while someone looks (E13-S03).
/// See the [module docs](super).
#[derive(Clone)]
pub struct MetricsService {
    inner: Arc<Inner>,
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
        let mut state = self.inner.state.lock();
        let config = state.config.get(cluster);
        state
            .clusters
            .entry(cluster.clone())
            .or_insert_with(|| Entry::new(config))
            .usage
            .clone()
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
            let config = state.config.get(cluster);
            let entry = state
                .clusters
                .entry(cluster.clone())
                .or_insert_with(|| Entry::new(config));
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
        MetricsInterest {
            inner: Arc::downgrade(&self.inner),
            cluster: cluster.clone(),
            kind,
        }
    }

    /// A handle a window moves to the cluster tab it shows ([`MetricsVisibility::show`]): a
    /// cluster no window shows is not polled. Dropping it shows nothing.
    pub fn visibility(&self) -> MetricsVisibility {
        MetricsVisibility {
            inner: Arc::downgrade(&self.inner),
            shown: None,
        }
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

/// A consumer of one kind of one cluster's metrics. Dropping it unregisters the consumer; the
/// last one gone stops the poller (its task is dropped, so the call in flight is aborted).
#[must_use = "the cluster is polled only while the interest is held"]
pub struct MetricsInterest {
    inner: Weak<Inner>,
    cluster: ClusterId,
    kind: MetricsKind,
}

impl std::fmt::Debug for MetricsInterest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsInterest")
            .field("cluster", &self.cluster)
            .field("kind", &self.kind)
            .finish()
    }
}

impl Drop for MetricsInterest {
    fn drop(&mut self) {
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        if let Some(entry) = inner.state.lock().clusters.get_mut(&self.cluster) {
            let count = &mut entry.interest[self.kind.index()];
            *count = count.saturating_sub(1);
        }
        inner.reconcile(&self.cluster);
    }
}

/// Which cluster tab one window shows. See [`MetricsService::visibility`].
pub struct MetricsVisibility {
    inner: Weak<Inner>,
    shown: Option<ClusterId>,
}

impl std::fmt::Debug for MetricsVisibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsVisibility")
            .field("shown", &self.shown)
            .finish()
    }
}

impl MetricsVisibility {
    /// The window now shows `cluster`'s tab (`None`: no cluster tab, the catalog say).
    pub fn show(&mut self, cluster: Option<ClusterId>) {
        if self.shown == cluster {
            return;
        }
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let previous = std::mem::replace(&mut self.shown, cluster.clone());
        {
            let mut state = inner.state.lock();
            if let Some(entry) = previous.as_ref().and_then(|c| state.clusters.get_mut(c)) {
                entry.visible = entry.visible.saturating_sub(1);
            }
            if let Some(cluster) = &cluster {
                let config = state.config.get(cluster);
                state
                    .clusters
                    .entry(cluster.clone())
                    .or_insert_with(|| Entry::new(config))
                    .visible += 1;
            }
        }
        for cluster in previous.iter().chain(cluster.iter()) {
            inner.reconcile(cluster);
        }
    }

    /// The cluster shown now.
    pub fn shown(&self) -> Option<&ClusterId> {
        self.shown.as_ref()
    }
}

impl Drop for MetricsVisibility {
    fn drop(&mut self) {
        self.show(None);
    }
}

impl Inner {
    pub(super) fn notify(&self, cluster: &ClusterId) {
        let _ = self.changes.send(cluster.clone());
    }

    fn reconcile_all(self: &Arc<Self>) {
        let clusters: Vec<ClusterId> = self.state.lock().clusters.keys().cloned().collect();
        for cluster in clusters {
            self.reconcile(&cluster);
        }
    }

    /// Starts `cluster`'s poller when it is watched, shown and connected, and stops it when it is
    /// not.
    pub(super) fn reconcile(self: &Arc<Self>, cluster: &ClusterId) {
        let connected = self
            .sessions
            .get(cluster)
            .is_some_and(|session| session.is_connected());
        let mut state = self.state.lock();
        let spawner = state.spawner.clone();
        let Some(entry) = state.clusters.get_mut(cluster) else {
            return;
        };
        let watched = entry.visible > 0 && entry.wanted().next().is_some() && connected;
        match (watched, entry.poller.is_some(), spawner) {
            (true, false, Some(spawner)) => {
                for cadence in &mut entry.cadence {
                    cadence.resume();
                }
                let task = poll_loop(Arc::downgrade(self), cluster.clone());
                entry.poller = Some(spawn_guarded(&spawner, task));
            }
            (false, true, _) => entry.poller = None,
            _ => {}
        }
    }

    /// The cluster dropped its connection (or closed): stop, forget its samples and history.
    fn disconnected(self: &Arc<Self>, cluster: &ClusterId) {
        let usage = {
            let mut state = self.state.lock();
            let Some(entry) = state.clusters.get_mut(cluster) else {
                return;
            };
            entry.poller = None;
            entry.cadence = Default::default();
            entry.usage.clone()
        };
        usage.clear();
        self.notify(cluster);
    }

    /// The namespace selection moved: the pods are read again, now.
    fn namespaces_changed(self: &Arc<Self>, cluster: &ClusterId) {
        let usage = {
            let mut state = self.state.lock();
            let Some(entry) = state.clusters.get_mut(cluster) else {
                return;
            };
            entry.cadence[MetricsKind::Pods.index()] = Cadence::default();
            if entry.poller.is_some() {
                entry.poller = None;
            }
            entry.usage.clone()
        };
        usage.clear_pods();
        self.notify(cluster);
        self.reconcile(cluster);
    }
}

/// Reacts to the session updates until the service is dropped.
async fn follow(inner: Weak<Inner>, mut updates: SessionUpdates) {
    while let Some(item) = updates.next().await {
        let Some(inner) = inner.upgrade() else {
            return;
        };
        match item {
            Ok(update) => match update.change {
                SessionChange::StateChanged { from, state } => {
                    let connected = state.phase().is_connected();
                    if !connected && from.is_connected() {
                        inner.disconnected(&update.cluster);
                    } else if connected && !from.is_connected() {
                        inner.reconcile(&update.cluster);
                    }
                }
                SessionChange::Closed => inner.disconnected(&update.cluster),
                SessionChange::NamespaceChanged(_) => inner.namespaces_changed(&update.cluster),
                _ => {}
            },
            Err(_lagged) => inner.reconcile_all(),
        }
    }
}
