//! [`MetricsProviderService`]: per-cluster detection state, run on connect and on demand.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::{FutureExt as _, StreamExt as _};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    DetectedProvider, DetectionState, MetricsMode, PromEndpoint, PrometheusOverride,
    ProviderDetectorPort,
};
use parking_lot::Mutex;
use tokio::sync::broadcast;

use super::run::{Found, detect};
use super::status::{ProviderState, ProviderStatus};
use crate::session::{ClusterSession, ClusterSessionManager, SessionChange, SessionUpdates};
use crate::store::{Spawner, TaskGuard, spawn_guarded};

/// A provider that answered and the endpoint queries to it go to (for a configured endpoint, with
/// the credentials' keychain references; for a detected one, a Service reached through the API
/// server).
#[derive(Debug, Clone)]
pub struct Resolved {
    /// The provider, as the page shows it.
    pub provider: DetectedProvider,
    /// Where to send queries.
    pub endpoint: PromEndpoint,
}

/// Where the provider that answered goes: the query side. The binary implements it over
/// `oxikube_prometheus::PrometheusPort`, so the endpoint a query uses is set in the same step as
/// the state a view shows. `None` forgets the cluster's endpoint.
pub trait EndpointSink: Send + Sync {
    /// `cluster`'s provider is now `resolved`, or none.
    fn publish(&self, cluster: &ClusterId, resolved: Option<&Resolved>);
}

/// An [`EndpointSink`] that drops everything (tests, and a build without a query side).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSink;

impl EndpointSink for NoSink {
    fn publish(&self, _: &ClusterId, _: Option<&Resolved>) {}
}

/// Capacity of the change broadcast. A subscriber that falls behind re-reads the status, which
/// is all a change means.
const CHANGE_CAPACITY: usize = 64;

struct Entry {
    status: Arc<ProviderStatus>,
    /// The settings and connection the status was computed for: a different one re-runs.
    basis: Basis,
    /// Which run owns the status; a finished run of an older number is ignored.
    run: u64,
    /// The run in flight. Replaced (and so aborted) by the next run; never dropped by itself.
    _task: Option<TaskGuard>,
}

/// What a status was computed from. A change of it runs detection again; a change of the rest of
/// the settings (the stack's name for the query catalogue, say) only updates what is shown.
#[derive(Clone, PartialEq, Eq)]
struct Basis {
    mode: MetricsMode,
    /// The settings that make up a configured endpoint; `None` unless the mode is manual.
    configured: Option<PrometheusOverride>,
    connected: bool,
}

impl Basis {
    fn of(prefs: &PrometheusOverride, connected: bool) -> Self {
        let mode = prefs.effective_mode();
        Self {
            mode,
            configured: (mode == MetricsMode::Manual).then(|| prefs.clone()),
            connected,
        }
    }
}

#[derive(Default)]
struct State {
    spawner: Option<Arc<dyn Spawner>>,
    follower: Option<TaskGuard>,
    entries: HashMap<ClusterId, Entry>,
    next_run: u64,
}

struct Inner {
    sessions: ClusterSessionManager,
    detector: Arc<dyn ProviderDetectorPort>,
    sink: Arc<dyn EndpointSink>,
    state: Mutex<State>,
    changes: broadcast::Sender<ClusterId>,
}

/// Finds and remembers each cluster's metrics provider. See the [module docs](super).
#[derive(Clone)]
pub struct MetricsProviderService {
    inner: Arc<Inner>,
}

impl MetricsProviderService {
    /// A service over `sessions` that detects with `detector` and publishes what it finds to
    /// `sink`. Nothing runs until [`start`](Self::start).
    pub fn new(
        sessions: ClusterSessionManager,
        detector: Arc<dyn ProviderDetectorPort>,
        sink: Arc<dyn EndpointSink>,
    ) -> Self {
        let (changes, _) = broadcast::channel(CHANGE_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                sessions,
                detector,
                sink,
                state: Mutex::new(State::default()),
                changes,
            }),
        }
    }

    /// Lets the service run detection on `spawner` (the Tokio bridge) and looks at the clusters
    /// that are connected already. Spawns nothing itself: the session updates are followed by the
    /// future [`follow`](Self::follow) returns, which the host runs where it likes. Safe to call
    /// again: the first spawner stays.
    pub fn attach(&self, spawner: Arc<dyn Spawner>) {
        {
            let mut state = self.inner.state.lock();
            if state.spawner.is_some() {
                return;
            }
            state.spawner = Some(spawner);
        }
        self.inner.evaluate_all(false);
    }

    /// The loop that follows the session updates: every cluster that connects from now on is
    /// detected (or its configured endpoint checked), a cluster that drops is forgotten. It
    /// subscribes now and runs until the service is dropped; it only waits and, per update, does a
    /// short in-memory evaluation, so a host runs it on its foreground executor and an idle app has
    /// no task on the runtime for it.
    pub fn follow(&self) -> BoxFuture<'static, ()> {
        let updates = self.inner.sessions.subscribe();
        follow(Arc::downgrade(&self.inner), updates).boxed()
    }

    /// [`attach`](Self::attach) and [`follow`](Self::follow) on `spawner`, for a host with no
    /// executor of its own. The follower's task is held by the service and aborted with it.
    pub fn start(&self, spawner: Arc<dyn Spawner>) {
        if self.inner.state.lock().follower.is_some() {
            return;
        }
        // Subscribe before looking at the connected clusters, so a connect in between is seen.
        let follower = self.follow();
        self.attach(spawner.clone());
        self.inner.state.lock().follower = Some(spawn_guarded(&spawner, follower));
    }

    /// The status of `cluster`. A cluster nothing was run for reads as `Disabled` (its settings
    /// turn metrics off) or `WaitingForCluster`.
    pub fn status(&self, cluster: &ClusterId) -> Arc<ProviderStatus> {
        if let Some(entry) = self.inner.state.lock().entries.get(cluster) {
            return entry.status.clone();
        }
        let prefs = self
            .inner
            .sessions
            .get(cluster)
            .map(|session| prefs_of(&session))
            .unwrap_or_else(|| {
                self.inner
                    .sessions
                    .cluster_prefs(cluster)
                    .prometheus
                    .clone()
                    .unwrap_or_default()
            });
        let mode = prefs.effective_mode();
        let state = if mode == MetricsMode::Off {
            ProviderState::Disabled
        } else {
            ProviderState::WaitingForCluster
        };
        Arc::new(ProviderStatus::new(mode, state).with_settings(prefs))
    }

    /// A stream with one item each time `cluster`'s status changes. Re-read it with
    /// [`status`](Self::status). Ends when the service is dropped.
    pub fn changes(&self, cluster: &ClusterId) -> BoxStream<'static, ()> {
        let cluster = cluster.clone();
        let rx = self.inner.changes.subscribe();
        futures::stream::unfold(rx, |mut rx| async move {
            match rx.recv().await {
                Ok(changed) => Some((Some(changed), rx)),
                // Missed some: the status may have moved, so wake the reader once.
                Err(broadcast::error::RecvError::Lagged(_)) => Some((None, rx)),
                Err(broadcast::error::RecvError::Closed) => None,
            }
        })
        .filter_map(move |changed| {
            let wanted = changed.as_ref().is_none_or(|changed| *changed == cluster);
            futures::future::ready(wanted.then_some(()))
        })
        .boxed()
    }

    /// Runs detection for `cluster` again, replacing a run in progress. A disconnected or
    /// disabled cluster keeps its status (there is nothing to probe).
    ///
    /// # Errors
    ///
    /// `NotFound` when no session is open for `cluster`; `Unsupported` before
    /// [`start`](Self::start).
    pub fn redetect(&self, cluster: &ClusterId) -> OxiResult<Arc<ProviderStatus>> {
        if self.inner.sessions.get(cluster).is_none() {
            return Err(OxiError::not_found(format!(
                "cluster {cluster} is not open, so there is no metrics provider to look for"
            )));
        }
        if self.inner.state.lock().spawner.is_none() {
            return Err(OxiError::unsupported(
                "metrics provider detection has not started",
            ));
        }
        self.inner.evaluate(cluster, true);
        Ok(self.status(cluster))
    }

    /// Whether `cluster`'s settings have history metrics on (the default). Reads the session's
    /// settings; a cluster with no session reads the default.
    pub fn metrics_enabled(&self, cluster: &ClusterId) -> bool {
        self.inner
            .sessions
            .cluster_prefs(cluster)
            .prometheus
            .as_ref()
            .is_none_or(|p| p.enabled)
    }

    /// The kubeconfig context name of `cluster`'s session, if one is open.
    pub fn context_name(&self, cluster: &ClusterId) -> Option<String> {
        self.inner
            .sessions
            .get(cluster)
            .map(|session| session.context().as_str().to_owned())
    }

    /// Looks at every cluster again after its settings may have changed: only the ones whose
    /// `prometheus` settings differ from what their status was computed for run again. Cheap
    /// when nothing changed, so the binary calls it on every settings change.
    pub fn settings_changed(&self) {
        self.inner.evaluate_all(false);
    }
}

impl std::fmt::Debug for MetricsProviderService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsProviderService")
            .field("clusters", &self.inner.state.lock().entries.len())
            .finish_non_exhaustive()
    }
}

fn prefs_of(session: &ClusterSession) -> PrometheusOverride {
    session.prefs().prometheus.clone().unwrap_or_default()
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
                    // A new connection (first, or after a drop) is a fresh look; Ready to
                    // Degraded and back is the same connection.
                    if connected && !from.is_connected() {
                        inner.evaluate(&update.cluster, true);
                    } else if !connected && from.is_connected() {
                        inner.evaluate(&update.cluster, false);
                    }
                }
                SessionChange::Closed => inner.forget(&update.cluster),
                _ => {}
            },
            Err(_lagged) => inner.evaluate_all(false),
        }
    }
}

impl Inner {
    fn evaluate_all(self: &Arc<Self>, force: bool) {
        for session in self.sessions.sessions() {
            self.evaluate(session.id(), force);
        }
    }

    fn forget(&self, cluster: &ClusterId) {
        let removed = self.state.lock().entries.remove(cluster);
        if removed.is_some() {
            self.sink.publish(cluster, None);
            let _ = self.changes.send(cluster.clone());
        }
    }

    /// Brings `cluster`'s status in line with its session and settings, starting a run when it
    /// must. `force` runs even when nothing changed (a reconnect, or the user's ask).
    fn evaluate(self: &Arc<Self>, cluster: &ClusterId, force: bool) {
        let Some(session) = self.sessions.get(cluster) else {
            self.forget(cluster);
            return;
        };
        let prefs = prefs_of(&session);
        let mode = prefs.effective_mode();
        let connected = session.is_connected();
        let basis = Basis::of(&prefs, connected);

        let mut guard = self.state.lock();
        let state = &mut *guard;
        if !force
            && let Some(entry) = state.entries.get_mut(cluster)
            && entry.basis == basis
        {
            // Nothing to run again; the settings the page shows may still have moved.
            if entry.status.settings != prefs {
                let mut status = (*entry.status).clone();
                status.settings = prefs;
                entry.status = Arc::new(status);
                drop(guard);
                let _ = self.changes.send(cluster.clone());
            }
            return;
        }
        // Without a spawner nothing can run: record the settings-only states, keep the rest.
        let spawner = state.spawner.clone();
        state.next_run += 1;
        let run = state.next_run;

        let (initial, work) = match (mode, connected, spawner) {
            (MetricsMode::Off, _, _) => (ProviderState::Disabled, None),
            (_, false, _) | (_, true, None) => (ProviderState::WaitingForCluster, None),
            (_, true, Some(spawner)) => (
                ProviderState::Detection(DetectionState::Detecting),
                Some(spawner),
            ),
        };
        let settings = prefs.clone();
        let task = work.and_then(|spawner| {
            let resources = session.resources()?;
            let proxy = session.service_proxy()?;
            let weak = Arc::downgrade(self);
            let detector = self.detector.clone();
            let cluster = cluster.clone();
            Some(spawn_guarded(&spawner, async move {
                let finished = detect(detector, mode, prefs, resources, proxy).await;
                if let Some(inner) = weak.upgrade() {
                    inner.finish(&cluster, run, mode, finished);
                }
            }))
        });
        // A connected session always has its ports; if it lost them in between, wait.
        let initial = if matches!(initial, ProviderState::Detection(_)) && task.is_none() {
            ProviderState::WaitingForCluster
        } else {
            initial
        };
        let status = ProviderStatus::new(mode, initial).with_settings(settings);
        // A run in progress keeps the endpoint the last one found until its own result lands.
        let publish = !status.is_detecting();
        state.entries.insert(
            cluster.clone(),
            Entry {
                status: Arc::new(status),
                basis,
                run,
                _task: task,
            },
        );
        drop(guard);
        if publish {
            self.sink.publish(cluster, None);
        }
        let _ = self.changes.send(cluster.clone());
    }

    /// A run ended: record its result unless a newer run (or a forget) superseded it.
    fn finish(&self, cluster: &ClusterId, run: u64, mode: MetricsMode, found: Found) {
        let Found { result, endpoint } = found;
        let status = {
            let mut state = self.state.lock();
            let Some(entry) = state.entries.get_mut(cluster).filter(|e| e.run == run) else {
                return;
            };
            let status = Arc::new(ProviderStatus {
                mode,
                state: ProviderState::Detection(result.state),
                probes: result.probes,
                checked_at: result.finished_at,
                settings: entry.status.settings.clone(),
            });
            entry.status = status.clone();
            status
        };
        let resolved = status
            .provider()
            .cloned()
            .zip(endpoint)
            .map(|(provider, endpoint)| Resolved { provider, endpoint });
        self.sink.publish(cluster, resolved.as_ref());
        let _ = self.changes.send(cluster.clone());
    }
}
