//! `DiscoveryPort` over the Kubernetes discovery endpoints.
//!
//! [`KubeDiscovery`] runs discovery on a `kube::Client`, keeps the result as a shared
//! [`Registry`] snapshot and answers [`DiscoveryPort`] calls from it:
//!
//! - **Discovery** reads aggregated discovery (two requests, Kubernetes 1.26+) and falls back to
//!   the legacy per-group endpoints when the server does not serve it (a legacy group version
//!   that fails to list keeps its previous kinds). See `convert` for how
//!   short names and categories are obtained (kube's `ApiResource` drops them).
//! - **`resolve`** is a cache lookup. A miss triggers one re-discovery (concurrent misses share
//!   it; a cooldown stops unknown kinds from hammering the server) before answering `None`.
//! - **CRD changes** are picked up by [`KubeDiscovery::watch_crds`]: a metadata-only watch on
//!   `CustomResourceDefinition`, debounced, re-runs discovery and publishes a [`RegistryDiff`]
//!   to [`KubeDiscovery::registry_changes`] receivers, so new CRDs appear without reconnecting.
//!   The app starts it through [`DiscoveryPort::subscribe`] (`events`). A user who may not watch
//!   CRDs gets [`CrdWatchStatus::Forbidden`] and a slow re-discovery instead of a silent retry
//!   loop. Aggregated APIs (`APIService`) are not watched; they are picked up by the next
//!   refresh or resolve miss.
//!
//! kube types stay inside the adapter; E04 reaches `ApiResource` through
//! [`KubeDiscovery::resolve_api_resource`].

mod convert;
mod crd_watch;
mod events;
mod fetch;
mod registry;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use kube::Client;
use kube::core::ApiResource;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::ResourceKind;
use oxikube_ports::{CrdWatchStatus, DiscoveryEvents, DiscoveryPort, ServerVersion};
use parking_lot::RwLock;
use tokio::sync::{Mutex, broadcast, watch};
use tokio::time::Instant;
use tracing::debug;

use crate::auth::classify;

pub use crd_watch::{CrdWatch, CrdWatchConfig};
pub use registry::{KindChange, Registry, RegistryDiff};

/// Registry diffs buffered per subscriber before the slowest one starts lagging.
const CHANGE_CHANNEL_CAPACITY: usize = 16;

/// Tuning for [`KubeDiscovery`].
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// Minimum time between re-discoveries triggered by `resolve` misses. A miss inside the
    /// window answers from the current registry. Default 2 s.
    pub miss_cooldown: Duration,
    /// Try aggregated discovery first (default). `false` always uses the legacy per-group
    /// endpoints: for servers or proxies that mishandle aggregated discovery, and for comparing
    /// the two.
    pub aggregated: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            miss_cooldown: Duration::from_secs(2),
            aggregated: true,
        }
    }
}

/// Kind discovery for one cluster. Cheap to clone; clones share the registry and the change
/// channel.
#[derive(Clone)]
pub struct KubeDiscovery {
    client: Client,
    shared: Arc<Shared>,
}

struct Shared {
    config: DiscoveryConfig,
    registry: RwLock<Registry>,
    /// Bumped after every completed refresh; lets waiters see that someone else refreshed.
    generation: AtomicU64,
    /// Serialises refreshes; holds when the last `resolve` miss triggered one (for the cooldown).
    refresh: Mutex<Option<Instant>>,
    changes: broadcast::Sender<Arc<RegistryDiff>>,
    /// Whether the CRD watch runs or was refused; see [`KubeDiscovery::crd_watch_status`].
    crd_status: watch::Sender<CrdWatchStatus>,
}

impl KubeDiscovery {
    /// Discovery over `client` with default tuning. Nothing is requested until the first
    /// [`discover`](DiscoveryPort::discover), [`refresh`](Self::refresh) or `resolve`.
    pub fn new(client: Client) -> Self {
        Self::with_config(client, DiscoveryConfig::default())
    }

    /// Discovery over `client` with explicit tuning.
    pub fn with_config(client: Client, config: DiscoveryConfig) -> Self {
        let (changes, _) = broadcast::channel(CHANGE_CHANNEL_CAPACITY);
        Self {
            client,
            shared: Arc::new(Shared {
                config,
                registry: RwLock::new(Registry::empty()),
                generation: AtomicU64::new(0),
                refresh: Mutex::new(None),
                changes,
                crd_status: watch::channel(CrdWatchStatus::Watching).0,
            }),
        }
    }

    /// The current registry snapshot (empty before the first discovery). Cheap.
    pub fn registry(&self) -> Registry {
        self.shared.registry.read().clone()
    }

    /// Receives registry changes. Every refresh that changes the registry (the first
    /// discovery counts: everything is `added`) publishes its diff; an unchanged refresh publishes
    /// nothing. A receiver that falls behind gets `RecvError::Lagged` and should re-read
    /// [`registry`](Self::registry).
    pub fn registry_changes(&self) -> broadcast::Receiver<Arc<RegistryDiff>> {
        self.shared.changes.subscribe()
    }

    /// Whether the CRD watch is running or the server refused it (`Forbidden`). `Watching` until
    /// a watch says otherwise, also before [`watch_crds`](Self::watch_crds) was called.
    pub fn crd_watch_status(&self) -> CrdWatchStatus {
        self.shared.crd_status.borrow().clone()
    }

    /// Sets the CRD watch status; receivers of [`DiscoveryPort::subscribe`] hear about changes only.
    pub(super) fn set_crd_watch_status(&self, status: CrdWatchStatus) {
        self.shared.crd_status.send_if_modified(|current| {
            let changed = *current != status;
            if changed {
                *current = status;
            }
            changed
        });
    }

    /// Re-runs discovery, swaps in the new registry and publishes the diff. Concurrent calls queue
    /// behind one another.
    pub async fn refresh(&self) -> OxiResult<Registry> {
        let _guard = self.shared.refresh.lock().await;
        self.refresh_locked().await
    }

    /// Whether the server serves any kind in API group `group`, discovering first when nothing has
    /// been discovered yet. Safe to call beside [`discover`](DiscoveryPort::discover): the two
    /// share the refresh lock, so a caller that arrives while the first discovery runs waits for
    /// it and reuses its result instead of starting another. A discovery failure answers `false`
    /// (the group is not known to be served).
    pub async fn serves_group(&self, group: &str) -> bool {
        self.ensure_discovered().await;
        self.registry()
            .kinds()
            .any(|kind| kind.gvk.group.as_ref() == group)
    }

    /// Runs the first discovery unless one has completed (or is in flight, in which case this
    /// waits for it).
    async fn ensure_discovered(&self) {
        if self.shared.generation.load(Ordering::Acquire) != 0 {
            return;
        }
        let _guard = self.shared.refresh.lock().await;
        if self.shared.generation.load(Ordering::Acquire) != 0 {
            return;
        }
        if let Err(error) = self.refresh_locked().await {
            debug!(%error, "discovery: first refresh failed");
        }
    }

    /// kube's `ApiResource` for `gvk`, for building dynamic `Api` handles. Same lookup, miss and
    /// error rules as [`DiscoveryPort::resolve`]. Adapter-internal (E04).
    pub async fn resolve_api_resource(&self, gvk: &Gvk) -> OxiResult<Option<ApiResource>> {
        self.lookup(|registry| registry.api_resource(gvk).cloned())
            .await
    }

    /// The refresh itself; callers hold the `refresh` lock.
    async fn refresh_locked(&self) -> OxiResult<Registry> {
        let started = Instant::now();
        let mut fetched = fetch::fetch(&self.client, self.shared.config.aggregated).await?;
        if !fetched.failed.is_empty() {
            // A group version that failed to list (an aggregated API blinking) keeps its last
            // known kinds instead of being reported removed and then added again.
            let known = self.registry().entries_for(&fetched.failed);
            fetched.kinds.extend(known);
        }
        let next = Registry::from_discovered(fetched.kinds);
        let previous = std::mem::replace(&mut *self.shared.registry.write(), next.clone());
        self.shared.generation.fetch_add(1, Ordering::AcqRel);

        let diff = previous.diff(&next);
        debug!(
            kinds = next.len(),
            aggregated = fetched.aggregated,
            elapsed_ms = started.elapsed().as_millis() as u64,
            added = diff.added.len(),
            removed = diff.removed.len(),
            changed = diff.changed.len(),
            "discovery: refreshed"
        );
        if !diff.is_empty() {
            // No subscribers is fine.
            let _ = self.shared.changes.send(Arc::new(diff));
        }
        Ok(next)
    }

    /// Looks `pick` up in the registry; on a miss re-discovers once (unless someone else just
    /// did, or the cooldown has not elapsed) and looks again.
    async fn lookup<T>(&self, pick: impl Fn(&Registry) -> Option<T>) -> OxiResult<Option<T>> {
        if let Some(hit) = pick(&self.registry()) {
            return Ok(Some(hit));
        }
        let seen = self.shared.generation.load(Ordering::Acquire);
        let mut last_miss = self.shared.refresh.lock().await;
        // A refresh that completed while we queued (a sibling's miss, the CRD watcher) is the
        // re-discovery this miss asked for.
        let already_refreshed = self.shared.generation.load(Ordering::Acquire) != seen;
        let cooling = last_miss.is_some_and(|at| at.elapsed() < self.shared.config.miss_cooldown);
        if !already_refreshed && !cooling {
            *last_miss = Some(Instant::now());
            self.refresh_locked().await?;
        }
        drop(last_miss);
        Ok(pick(&self.registry()))
    }
}

impl std::fmt::Debug for KubeDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KubeDiscovery")
            .field("registry", &self.registry())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl DiscoveryPort for KubeDiscovery {
    async fn discover(&self) -> OxiResult<Vec<ResourceKind>> {
        Ok(self.refresh().await?.kinds().cloned().collect())
    }

    async fn resolve(&self, kind: &Gvk) -> OxiResult<Option<ResourceKind>> {
        self.lookup(|registry| registry.get(kind).cloned()).await
    }

    async fn server_version(&self) -> OxiResult<ServerVersion> {
        let info = self
            .client
            .apiserver_version()
            .await
            .map_err(|e| classify(&e))?;
        Ok(ServerVersion {
            major: info.major,
            minor: info.minor,
            git_version: info.git_version,
            platform: info.platform,
        })
    }

    fn subscribe(&self) -> DiscoveryEvents {
        self.events()
    }
}
