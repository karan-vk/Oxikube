//! [`PortForwardManager`]: the registry of forwards across clusters, the read-only policy, the
//! notifications and the coalesced change subscription. Each running forward is one supervisor
//! task (`supervisor`) owned by its entry and aborted when the forward stops, is removed or
//! the manager is dropped.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::portforward::ForwardSpec;
use oxikube_ports::{ClockPort, ForwardStatsSnapshot, NotificationSourcePort};
use parking_lot::Mutex;
use tokio::sync::watch;

use super::backend::ForwardBackend;
use super::model::{ForwardConfig, ForwardId, ForwardInfo, StartError};
use super::registry::{Entry, FavouriteSaver, Inner};
use super::state::{ForwardEvent, ForwardState};
use super::supervisor::Supervisor;
use crate::store::{Spawner, spawn_guarded};

/// How long [`PortForwardManager::start_and_wait`] waits for a forward to leave `Starting`.
pub const START_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// Runs port-forwards to pods and Services across clusters and keeps them alive through pod
/// churn. See the [module docs](super).
///
/// Cheap to clone (shared state). Dropping the last clone stops every forward.
#[derive(Clone)]
pub struct PortForwardManager {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PortForwardManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PortForwardManager")
            .field("forwards", &self.inner.entries.lock().len())
            .finish_non_exhaustive()
    }
}

/// The coalesced change stream of a [`PortForwardManager`]: [`changed`](Self::changed) wakes once
/// however many changes happened since the last wake, so a panel redraws at most once per wake.
/// Traffic counters are not changes; sample them with [`PortForwardManager::list`] on a timer.
#[derive(Debug, Clone)]
pub struct ForwardChanges(watch::Receiver<u64>);

impl ForwardChanges {
    /// A change stream over `changes`, for a fake that is not a manager.
    pub fn new(changes: watch::Receiver<u64>) -> Self {
        Self(changes)
    }

    /// Waits for the next change; `false` once the manager is gone.
    pub async fn changed(&mut self) -> bool {
        self.0.changed().await.is_ok()
    }
}

impl PortForwardManager {
    /// A manager over `backend`, running its supervisors on `spawner` and its backoff timers on
    /// `clock`; drops, recoveries and failures go to `notify` (the notification centre's hooks).
    pub fn new(
        backend: Arc<dyn ForwardBackend>,
        spawner: Arc<dyn Spawner>,
        clock: Arc<dyn ClockPort>,
        notify: Option<Arc<dyn NotificationSourcePort>>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                backend,
                spawner,
                clock,
                notify,
                config: Mutex::new(ForwardConfig::default()),
                clusters: Mutex::new(HashMap::new()),
                entries: Mutex::new(BTreeMap::new()),
                next_id: AtomicU64::new(1),
                changes: watch::channel(0).0,
                favourite_saver: Mutex::new(None),
            }),
        }
    }

    /// Installs how a forward started with `favourite` is saved (the favourites do this).
    pub(super) fn set_favourite_saver(&self, saver: FavouriteSaver) {
        *self.inner.favourite_saver.lock() = Some(saver);
    }

    /// Saves `spec` as an auto-start favourite through the installed saver.
    ///
    /// # Errors
    ///
    /// An unsupported error when no favourites are wired, else the saver's own errors.
    pub(super) async fn save_favourite(&self, spec: ForwardSpec) -> oxikube_domain::OxiResult<()> {
        let saver = self.inner.favourite_saver.lock().clone();
        match saver {
            Some(save) => save(spec).await,
            None => Err(oxikube_domain::OxiError::unsupported(
                "port-forward favourites are not available",
            )),
        }
    }

    /// Replaces the configuration (the `portforward.*` settings); running forwards keep going.
    pub fn set_config(&self, config: ForwardConfig) {
        *self.inner.config.lock() = config;
    }

    /// Replaces the per-cluster overrides (`clusters.<id>.portforward`, already resolved over
    /// the global values). Clusters not listed follow [`config`](Self::config). Like
    /// [`set_config`](Self::set_config), only forwards started afterwards see it.
    pub fn set_cluster_configs(&self, clusters: HashMap<ClusterId, ForwardConfig>) {
        *self.inner.clusters.lock() = clusters;
    }

    /// The configuration in force for `cluster`: its override, else the global one. The backoff
    /// is the global one in both cases.
    pub fn config_for(&self, cluster: &ClusterId) -> ForwardConfig {
        let global = self.config();
        self.inner
            .clusters
            .lock()
            .get(cluster)
            .map_or(global, |c| ForwardConfig {
                backoff: global.backoff,
                ..*c
            })
    }

    /// The cluster side.
    pub(super) fn backend(&self) -> &Arc<dyn ForwardBackend> {
        &self.inner.backend
    }

    /// The configuration in force.
    pub fn config(&self) -> ForwardConfig {
        *self.inner.config.lock()
    }

    /// Why a forward could not start on `cluster` right now, if so: it is read-only and
    /// `portforward.allowed_in_readonly` is off for it. (Whether it is connected is learnt at the
    /// start.) Lets a caller refuse before asking anyone for permission.
    pub fn refusal(&self, cluster: &ClusterId) -> Option<StartError> {
        (self.inner.backend.is_read_only(cluster) && !self.config_for(cluster).allowed_in_readonly)
            .then(|| StartError::ReadOnly(cluster.to_string()))
    }

    /// Starts forwarding `spec` and returns its id at once; the forward resolves its target and
    /// binds in the background (watch it through [`subscribe`](Self::subscribe), or use
    /// [`start_and_wait`](Self::start_and_wait)).
    ///
    /// # Errors
    ///
    /// [`StartError::ReadOnly`] on a read-only cluster unless `allowed_in_readonly`,
    /// [`StartError::NotConnected`], [`StartError::Unsupported`] for a target that is not a
    /// namespaced pod or Service.
    pub fn start(&self, spec: ForwardSpec) -> Result<ForwardId, StartError> {
        check_target(&spec)?;
        let id = ForwardId(self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let entry = Entry {
            spec,
            state: ForwardState::Starting,
            local_addr: None,
            pod: None,
            last_error: None,
            stats: None,
            carried: ForwardStatsSnapshot::default(),
            generation: 0,
            task: None,
            stop: None,
            started_at: self.inner.clock.now(),
        };
        self.inner.entries.lock().insert(id, entry);
        if let Err(err) = self.launch(id) {
            self.inner.entries.lock().remove(&id);
            return Err(err);
        }
        Ok(id)
    }

    /// [`start`](Self::start), then waits until the forward has left `Starting`: `Active` (or
    /// already reconnecting: no ready pod yet) is `Ok`, `Failed` is the start's error, with its
    /// suggestion; a failed start is removed again, so a retry does not leave a dead duplicate. Gives up waiting (with the forward still
    /// `Starting`, as `Ok`, still listed) after [`START_WAIT`] of the manager's clock, so a command or a tool
    /// never hangs on a feed that does not list.
    ///
    /// # Errors
    ///
    /// As [`start`](Self::start), plus [`StartError::Forward`] / [`StartError::Failed`].
    pub async fn start_and_wait(&self, spec: ForwardSpec) -> Result<ForwardInfo, StartError> {
        let mut changes = self.subscribe();
        let id = self.start(spec)?;
        loop {
            let info = self.get(id).ok_or(StartError::Unknown(id))?;
            match &info.state {
                ForwardState::Starting => {}
                ForwardState::Failed(failure) => {
                    let err = match &failure.error {
                        Some(err) => StartError::Forward(err.clone()),
                        None => StartError::Failed(failure.message.clone()),
                    };
                    self.remove(id);
                    return Err(err);
                }
                _ => return Ok(info),
            }
            let changed = changes.changed();
            let timeout = self.inner.clock.sleep(START_WAIT);
            futures::pin_mut!(changed, timeout);
            match futures::future::select(changed, timeout).await {
                futures::future::Either::Left((true, _)) => {}
                futures::future::Either::Left((false, _)) => return Err(StartError::Unknown(id)),
                futures::future::Either::Right(_) => return Ok(info),
            }
        }
    }

    /// Stops forward `id`; it stays listed as `Stopped`. Returns whether there was one.
    ///
    /// The stop is graceful: the listener stops accepting at once, connections in flight get the
    /// adapter's grace period to finish (a download or a transaction ends cleanly instead of
    /// being reset), and whatever is left after [`STOP_GRACE`](super::supervisor::STOP_GRACE)
    /// is aborted.
    pub fn stop(&self, id: ForwardId) -> bool {
        self.halt(id, true)
    }

    /// Stops `id`'s supervisor: `graceful` signals it and gives it [`STOP_GRACE`] to drain before
    /// aborting it; otherwise it is aborted at once (a restart, which rebinds the same port).
    ///
    /// [`STOP_GRACE`]: super::supervisor::STOP_GRACE
    fn halt(&self, id: ForwardId, graceful: bool) -> bool {
        let mut running = None;
        let found = self
            .inner
            .transition(id, None, ForwardEvent::Stop, |entry| {
                running = entry.0.task.take().zip(entry.0.stop.take());
                entry.0.generation += 1;
            });
        if let Some((task, stop)) = running.filter(|_| graceful)
            && stop.send(()).is_ok()
        {
            let clock = self.inner.clock.clone();
            self.inner.spawner.spawn(Box::pin(async move {
                // Holding the guard keeps the supervisor alive while it drains; dropping it
                // after the grace aborts whatever is left.
                let _task = task;
                clock.sleep(super::supervisor::STOP_GRACE).await;
            }));
        }
        found
    }

    /// Starts forward `id` again from `Starting`, whatever its state, on the same local port.
    ///
    /// # Errors
    ///
    /// [`StartError::Unknown`], or the [`start`](Self::start) policy errors.
    pub fn restart(&self, id: ForwardId) -> Result<(), StartError> {
        self.halt(id, false)
            .then_some(())
            .ok_or(StartError::Unknown(id))?;
        let now = self.inner.clock.now();
        self.inner
            .transition(id, None, ForwardEvent::Restart, |e| e.0.started_at = now);
        self.launch(id)
    }

    /// Stops forward `id` and drops it from the list. Returns whether there was one.
    pub fn remove(&self, id: ForwardId) -> bool {
        let removed = self.stop(id) && self.inner.entries.lock().remove(&id).is_some();
        if removed {
            self.inner.bump();
        }
        removed
    }

    /// Every forward, by id, with its traffic sampled now.
    pub fn list(&self) -> Vec<ForwardInfo> {
        let entries = self.inner.entries.lock();
        entries.iter().map(|(id, e)| e.info(*id)).collect()
    }

    /// Forward `id`, if listed.
    pub fn get(&self, id: ForwardId) -> Option<ForwardInfo> {
        self.inner.entries.lock().get(&id).map(|e| e.info(id))
    }

    /// The coalesced change stream (state, address, pod or list changes).
    pub fn subscribe(&self) -> ForwardChanges {
        ForwardChanges(self.inner.changes.subscribe())
    }

    /// Checks the policy and spawns a supervisor for `id` (which is `Starting`).
    fn launch(&self, id: ForwardId) -> Result<(), StartError> {
        let (spec, local_port) = {
            let entries = self.inner.entries.lock();
            let entry = entries.get(&id).ok_or(StartError::Unknown(id))?;
            (
                entry.spec.clone(),
                entry.local_addr.map_or(entry.spec.local_port, |a| a.port()),
            )
        };
        let cluster = &spec.target.cluster;
        let refuse = |err: StartError| {
            let failure = super::state::ForwardFailure::message(err.to_string());
            self.inner.transition(id, None, ForwardEvent::Restart, |e| {
                e.0.state = ForwardState::Failed(failure);
            });
            Err(err)
        };
        let config = self.config_for(cluster);
        if let Some(err) = self.refusal(cluster) {
            return refuse(err);
        }
        let Some(port) = self.inner.backend.port_forward(cluster) else {
            return refuse(StartError::NotConnected(cluster.to_string()));
        };
        let Some(watch) = super::target_watch::watch_target(&*self.inner.backend, &spec) else {
            return refuse(StartError::NotConnected(cluster.to_string()));
        };
        let mut entries = self.inner.entries.lock();
        let Some(entry) = entries.get_mut(&id) else {
            return Err(StartError::Unknown(id));
        };
        entry.generation += 1;
        let supervisor = Supervisor {
            id,
            generation: entry.generation,
            spec,
            local_port: (local_port != 0).then_some(local_port),
            port_range: config.port_range,
            port,
            inner: Arc::downgrade(&self.inner),
            last_pod: entry.pod.clone(),
        };
        let (stop, stopped) = futures::channel::oneshot::channel();
        entry.stop = Some(stop);
        entry.task = Some(spawn_guarded(
            &self.inner.spawner,
            supervisor.run(watch, stopped),
        ));
        Ok(())
    }
}

fn check_target(spec: &ForwardSpec) -> Result<(), StartError> {
    let target = &spec.target;
    let core = target.gvk == Gvk::new("", "v1", "Pod") || spec.targets_service();
    if core && target.namespace.is_some() {
        Ok(())
    } else {
        Err(StartError::Unsupported(target.gvk.kind.to_string()))
    }
}
