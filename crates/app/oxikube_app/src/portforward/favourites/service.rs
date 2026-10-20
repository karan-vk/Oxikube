//! [`PortForwardFavourites`]: the favourites (saved through [`FavouriteStore`]) and the follower
//! that restores them when a cluster session becomes `Ready`.

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::future::{BoxFuture, FutureExt as _};
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::portforward::ForwardSpec;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ClockPort, NotificationSourcePort, StatePort};
use parking_lot::Mutex;

use super::model::{FavouriteForward, FavouriteId};
use super::restore::{self, RestoreReport};
use super::store::FavouriteStore;
use crate::portforward::PortForwardManager;
use crate::session::{ClusterSessionManager, SessionChange, SessionUpdate};
use crate::store::{Spawner, TaskGuard, spawn_guarded};

/// Saved port-forwards, restored when their cluster connects. See the [module docs](super).
///
/// Cheap to clone (shared state). Dropping the last clone aborts the restores in flight; the
/// forwards they already started belong to the [`PortForwardManager`] and keep running.
#[derive(Clone)]
pub struct PortForwardFavourites {
    pub(super) inner: Arc<Inner>,
}

pub(super) struct Inner {
    pub(super) store: FavouriteStore,
    pub(super) manager: PortForwardManager,
    pub(super) clock: Arc<dyn ClockPort>,
    pub(super) notify: Option<Arc<dyn NotificationSourcePort>>,
    spawner: Arc<dyn Spawner>,
    /// The restore running (or last run) per cluster; a new `Ready` replaces, and so aborts, it.
    pub(super) restores: Mutex<HashMap<ClusterId, TaskGuard>>,
}

impl std::fmt::Debug for PortForwardFavourites {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PortForwardFavourites")
            .finish_non_exhaustive()
    }
}

impl PortForwardFavourites {
    /// Favourites over `state`, started through `manager`; the restores run on `spawner`, and a
    /// favourite that could not start is told to `notify` (the notification centre's hooks).
    pub fn new(
        state: Arc<dyn StatePort>,
        manager: PortForwardManager,
        spawner: Arc<dyn Spawner>,
        clock: Arc<dyn ClockPort>,
        notify: Option<Arc<dyn NotificationSourcePort>>,
    ) -> Self {
        let favourites = Self {
            inner: Arc::new(Inner {
                store: FavouriteStore::new(state),
                manager: manager.clone(),
                clock,
                notify,
                spawner,
                restores: Mutex::new(HashMap::new()),
            }),
        };
        // `portforward::Start` with `favourite` (the dialog's "Keep as a favourite") saves
        // through here. Weak: the manager must not keep the favourites alive.
        let weak = Arc::downgrade(&favourites.inner);
        manager.set_favourite_saver(Arc::new(move |spec: ForwardSpec| {
            let weak = weak.clone();
            async move {
                let inner = weak
                    .upgrade()
                    .ok_or_else(|| OxiError::internal("port-forward favourites are gone"))?;
                let name = spec.name.clone();
                PortForwardFavourites { inner }
                    .add(spec, name, true)
                    .await
                    .map(drop)
            }
            .boxed()
        }));
        favourites
    }

    /// Every favourite.
    ///
    /// # Errors
    ///
    /// The state store's own errors.
    pub async fn list(&self) -> OxiResult<Vec<FavouriteForward>> {
        self.inner.store.list().await
    }

    /// The favourite forwarding `spec`, if saved (the panel's star).
    ///
    /// # Errors
    ///
    /// The state store's own errors.
    pub async fn find(&self, spec: &ForwardSpec) -> OxiResult<Option<FavouriteForward>> {
        self.inner.store.get(&FavouriteId::of(spec)).await
    }

    /// Saves `spec` as a favourite named `name` (or after the target). Saving a forward that is a
    /// favourite already updates its name and switch and keeps its date.
    ///
    /// # Errors
    ///
    /// A validation error for a target that is not a namespaced pod or Service, or the state
    /// store's own errors.
    pub async fn add(
        &self,
        spec: ForwardSpec,
        name: Option<String>,
        auto_start: bool,
    ) -> OxiResult<FavouriteForward> {
        let target = &spec.target;
        let forwardable = target.namespace.is_some()
            && (spec.targets_service() || target.gvk == Gvk::new("", "v1", "Pod"));
        if !forwardable {
            return Err(OxiError::validation(format!(
                "only pods and services can be saved as port-forward favourites, not {}",
                target.gvk.kind
            )));
        }
        let now = self.inner.clock.now().as_millisecond();
        let mut favourite = FavouriteForward::new(spec, name, now);
        favourite.auto_start = auto_start;
        if let Some(saved) = self.inner.store.get(&favourite.id).await? {
            favourite.created_ms = saved.created_ms;
        }
        self.inner.store.put(&favourite).await?;
        Ok(favourite)
    }

    /// Removes the favourite `id`; a forward running from it keeps running. Returns whether it
    /// was saved.
    ///
    /// # Errors
    ///
    /// The state store's own errors.
    pub async fn remove(&self, id: &FavouriteId) -> OxiResult<bool> {
        self.inner.store.remove(id).await
    }

    /// Flips the favourite `id`'s start-on-connect; returns the new value.
    ///
    /// # Errors
    ///
    /// A not-found error when `id` is not saved, or the state store's own errors.
    pub async fn toggle_auto_start(&self, id: &FavouriteId) -> OxiResult<bool> {
        let mut favourite = self
            .inner
            .store
            .get(id)
            .await?
            .ok_or_else(|| OxiError::not_found(format!("no port-forward favourite {id}")))?;
        favourite.auto_start = !favourite.auto_start;
        self.inner.store.put(&favourite).await?;
        Ok(favourite.auto_start)
    }

    /// Starts the auto-start favourites of `cluster` that are not forwarded yet, at most
    /// [`RESTORE_CONCURRENCY`](restore::RESTORE_CONCURRENCY) at a time, and tells the
    /// notification centre about each that could not start. The loop that follows the sessions
    /// calls this in the background; it is public for tests and for a "restore now" action.
    ///
    /// # Errors
    ///
    /// The state store's own errors (reading the favourites); a favourite that does not start is
    /// reported in the [`RestoreReport`], never an error.
    pub async fn restore(&self, cluster: &ClusterId) -> OxiResult<RestoreReport> {
        restore::restore(&self.inner, cluster).await
    }

    /// The loop that restores a cluster's favourites when its session becomes `Ready` (also for
    /// sessions that already are). It only waits and spawns, so a host runs it on its foreground
    /// executor; the restores themselves run on the spawner, never in the session's way.
    pub fn follow(&self, sessions: &ClusterSessionManager) -> BoxFuture<'static, ()> {
        let updates = sessions.subscribe();
        let sessions = sessions.clone();
        let inner = Arc::downgrade(&self.inner);
        async move {
            let ready = |sessions: &ClusterSessionManager| -> Vec<ClusterId> {
                sessions
                    .sessions()
                    .iter()
                    .filter(|s| s.phase() == SessionPhase::Ready)
                    .map(|s| s.id().clone())
                    .collect()
            };
            if let Some(inner) = inner.upgrade() {
                for cluster in ready(&sessions) {
                    schedule(&inner, cluster);
                }
            }
            let mut updates = updates;
            while let Some(update) = updates.next().await {
                let Some(inner) = inner.upgrade() else { return };
                match update {
                    Ok(SessionUpdate {
                        cluster,
                        change:
                            SessionChange::StateChanged {
                                from,
                                state: ClusterSessionState::Ready,
                            },
                    }) if from != SessionPhase::Degraded => schedule(&inner, cluster),
                    Ok(SessionUpdate {
                        cluster,
                        change: SessionChange::Closed,
                    }) => {
                        inner.restores.lock().remove(&cluster);
                    }
                    Ok(_) => {}
                    // Missed updates: restoring a cluster twice is harmless (it skips what runs).
                    Err(_) => {
                        for cluster in ready(&sessions) {
                            schedule(&inner, cluster);
                        }
                    }
                }
            }
        }
        .boxed()
    }
}

/// Restores `cluster`'s favourites in the background, replacing (aborting) a restore still
/// running for it.
fn schedule(inner: &Arc<Inner>, cluster: ClusterId) {
    let task = Arc::clone(inner);
    let key = cluster.clone();
    let guard = spawn_guarded(&inner.spawner, async move {
        if let Err(error) = restore::restore(&task, &cluster).await {
            tracing::warn!(%error, "port-forward favourites could not be read; none restored");
        }
    });
    inner.restores.lock().insert(key, guard);
}
