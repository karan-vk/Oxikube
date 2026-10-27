//! [`CloudDiscoveryService`]: refresh, cache, add and remove discovered clusters.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::stream::BoxStream;
use oxikube_domain::command::{KubeconfigSourceRef, NewKubeconfigSource};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{
    ClockPort, CloudDetection, CloudDiscoveryPort, CloudProvider, CloudScope, ClusterSourcePort,
    DiscoveredCluster, FsPort, StatePort,
};
use parking_lot::Mutex;

use super::cache::DiscoveryCache;
use super::event::{
    AddedCluster, CloudDiscoveryEvent, DiscoveryConfig, RefreshMode, RemovedCluster,
};
use super::managed::{ManagedEntry, ManagedFiles};
use super::run::{self, Run, RunEvents};
use crate::session::ClusterSessionManager;
use crate::sources::KubeconfigSourcesService;
use crate::store::{Spawner, TaskGuard, spawn_guarded};

/// What a [`CloudDiscoveryService`] is built from.
pub struct DiscoveryDeps {
    /// One adapter per provider (`oxikube_cloud` in the app). A provider without one is
    /// reported as not installed.
    pub providers: Vec<Arc<dyn CloudDiscoveryPort>>,
    /// The cache.
    pub state: Arc<dyn StatePort>,
    /// The managed files are written through it.
    pub fs: Arc<dyn FsPort>,
    /// The kubeconfig sources: the managed files are listed there, and its directory
    /// (`<config dir>/kubeconfigs`) holds them.
    pub sources: KubeconfigSourcesService,
    /// The catalog's contexts, to find the session of a removed cluster.
    pub source: Arc<dyn ClusterSourcePort>,
    /// The sessions, to disconnect a removed cluster.
    pub sessions: ClusterSessionManager,
    /// The cache's timestamps and TTL.
    pub clock: Arc<dyn ClockPort>,
    /// Runs the refreshes (the Tokio bridge in the app, the same runtime as `spawn_kube`).
    pub spawner: Arc<dyn Spawner>,
}

/// A refresh in flight: dropping it aborts the task.
struct ActiveRun {
    guard: TaskGuard,
    events: RunEvents,
}

/// Discovers cloud clusters through the [`CloudDiscoveryPort`]s and adds the chosen ones to
/// Oxikube's managed kubeconfigs. Cheap to clone; clones share runs, cache and locks.
///
/// See the [module docs](super) for the rules.
#[derive(Clone)]
pub struct CloudDiscoveryService {
    inner: Arc<Inner>,
}

struct Inner {
    providers: Vec<Arc<dyn CloudDiscoveryPort>>,
    cache: DiscoveryCache,
    managed: ManagedFiles,
    sources: KubeconfigSourcesService,
    source: Arc<dyn ClusterSourcePort>,
    sessions: ClusterSessionManager,
    spawner: Arc<dyn Spawner>,
    config: Mutex<DiscoveryConfig>,
    runs: Mutex<HashMap<CloudProvider, ActiveRun>>,
    /// Held across a read-modify-write of a managed file.
    write: futures::lock::Mutex<()>,
}

impl std::fmt::Debug for CloudDiscoveryService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudDiscoveryService")
            .field("providers", &self.providers())
            .finish_non_exhaustive()
    }
}

impl CloudDiscoveryService {
    /// A service over `deps`, with the default [`DiscoveryConfig`].
    pub fn new(deps: DiscoveryDeps) -> Self {
        let dir = deps.sources.kubeconfigs_dir().to_path_buf();
        Self {
            inner: Arc::new(Inner {
                providers: deps.providers,
                cache: DiscoveryCache::new(deps.state, deps.clock),
                managed: ManagedFiles::new(deps.fs, dir),
                sources: deps.sources,
                source: deps.source,
                sessions: deps.sessions,
                spawner: deps.spawner,
                config: Mutex::new(DiscoveryConfig::default()),
                runs: Mutex::default(),
                write: futures::lock::Mutex::new(()),
            }),
        }
    }

    /// The providers that have an adapter.
    pub fn providers(&self) -> Vec<CloudProvider> {
        let config = self.inner.config.lock();
        self.inner
            .providers
            .iter()
            .map(|p| p.id())
            .filter(|id| !config.disabled.contains(id))
            .collect()
    }

    /// Replaces the configuration (the settings changed). Runs started later use it.
    ///
    /// Providers newly switched off stop at once: their running refresh is cancelled.
    pub fn set_config(&self, config: DiscoveryConfig) {
        let newly_disabled: Vec<CloudProvider> = {
            let mut current = self.inner.config.lock();
            let newly = config
                .disabled
                .difference(&current.disabled)
                .copied()
                .collect();
            *current = config;
            newly
        };
        for provider in newly_disabled {
            self.cancel(provider);
        }
    }

    /// The configuration in use.
    pub fn config(&self) -> DiscoveryConfig {
        self.inner.config.lock().clone()
    }

    /// The managed kubeconfig of `provider`: `<config dir>/kubeconfigs/<cli>.yaml`.
    pub fn managed_path(&self, provider: CloudProvider) -> PathBuf {
        self.inner.managed.path(provider)
    }

    /// Refreshes `provider` in a task and returns its [`CloudDiscoveryEvent`]s. A refresh that is
    /// still running for the same provider is cancelled first (its stream ends with
    /// `Cancelled`).
    ///
    /// `scopes` narrows the run to those scopes (the catalog's filters); `None` lists the
    /// scopes the CLI reports. With [`RefreshMode::IfStale`] a cache within the TTL answers
    /// without running the CLI at all; an older one is sent first and the CLI runs after it.
    ///
    /// Returns at once: detection and listing run on the spawner, never on the caller's thread.
    pub fn refresh(
        &self,
        provider: CloudProvider,
        scopes: Option<Vec<CloudScope>>,
        mode: RefreshMode,
    ) -> BoxStream<'static, CloudDiscoveryEvent> {
        let (tx, rx) = mpsc::unbounded();
        let events = RunEvents::new(tx);
        if self.is_disabled(provider) {
            // Switched off in settings: nothing runs, nothing is reported as missing.
            events.end(CloudDiscoveryEvent::Finished {
                provider,
                clusters: Vec::new(),
                failed_scopes: 0,
            });
            return rx.boxed();
        }
        let Some(port) = self.port(provider) else {
            events.send(CloudDiscoveryEvent::Detected(
                CloudDetection::not_installed(provider),
            ));
            events.end(CloudDiscoveryEvent::Finished {
                provider,
                clusters: Vec::new(),
                failed_scopes: 0,
            });
            return rx.boxed();
        };
        self.cancel(provider);
        let run = Run {
            port,
            cache: self.inner.cache.clone(),
            config: self.config(),
            mode,
            scopes,
            events: events.clone(),
        };
        let guard = spawn_guarded(&self.inner.spawner, run.execute());
        self.inner
            .runs
            .lock()
            .insert(provider, ActiveRun { guard, events });
        rx.boxed()
    }

    /// Stops the refresh of `provider`, killing its CLI processes; its stream ends with
    /// `Cancelled`. Returns whether a refresh was running.
    pub fn cancel(&self, provider: CloudProvider) -> bool {
        let Some(run) = self.inner.runs.lock().remove(&provider) else {
            return false;
        };
        let ActiveRun { guard, events } = run;
        // Abort first, so the run cannot send another step after `Cancelled`.
        drop(guard);
        events.end(CloudDiscoveryEvent::Cancelled { provider })
    }

    /// Whether a refresh of `provider` is running.
    pub fn is_running(&self, provider: CloudProvider) -> bool {
        self.inner
            .runs
            .lock()
            .get(&provider)
            .is_some_and(|run| !run.events.has_ended())
    }

    /// The cached clusters of `provider` (every cached scope), whatever their age. Reads the
    /// state db only; no CLI runs.
    /// Empty for a provider switched off in settings.
    pub async fn cached(&self, provider: CloudProvider) -> Vec<DiscoveredCluster> {
        if self.is_disabled(provider) {
            return Vec::new();
        }
        let snapshot = self.inner.cache.load(provider).await;
        run::clusters_of(&snapshot, &snapshot.known_scopes())
    }

    /// The clusters in the managed kubeconfig of `provider`.
    ///
    /// # Errors
    ///
    /// As [`add`](Self::add) reading the file.
    pub async fn added(&self, provider: CloudProvider) -> OxiResult<Vec<ManagedEntry>> {
        Ok(self.inner.managed.read(provider).await?.entries().to_vec())
    }

    /// Adds `cluster` to its provider's managed kubeconfig (merged by context name, so adding
    /// it again changes nothing), lists that file as a kubeconfig source, and reloads the
    /// sources so the catalog shows it at once. The entry authenticates through the provider's
    /// exec plugin; no token is written. The user's own kubeconfig is not touched.
    ///
    /// Runs the CLI when the adapter needs it for the entry, so call it off the UI thread.
    ///
    /// # Errors
    ///
    /// `Unsupported` when no adapter serves the provider or its auth plugin is missing (from
    /// the adapter); `Conflict` when a file at the managed path was not written by cloud
    /// discovery; `Validation` when the managed file no longer parses; the filesystem's, the
    /// source list's or the cluster source's error. A failed write leaves the old file.
    pub async fn add(&self, cluster: &DiscoveredCluster) -> OxiResult<AddedCluster> {
        let provider = cluster.provider();
        let port = self.port(provider).ok_or_else(|| no_adapter(provider))?;
        let entry = port.kubeconfig_entry(cluster).await?;
        let _write = self.inner.write.lock().await;
        let mut file = self.inner.managed.read(provider).await?;
        let context = entry.context.clone();
        let changed = file.upsert(ManagedEntry {
            entry,
            cluster_key: Some(cluster.key()),
        });
        if changed {
            self.inner.managed.write(provider, &file).await?;
        }
        let path = self.managed_path(provider);
        // Listing the file reloads every source (also when it was listed already).
        self.inner
            .sources
            .add(&NewKubeconfigSource::File {
                path: path_text(&path),
            })
            .await?;
        Ok(AddedCluster {
            context,
            path,
            unchanged: !changed,
        })
    }

    /// Removes the cluster of `context` from the managed kubeconfig of `provider`, reloads the
    /// sources, and disconnects its session when one is open. The last cluster takes the file
    /// and its source entry with it.
    ///
    /// # Errors
    ///
    /// `NotFound` when the file holds no such context; otherwise as [`add`](Self::add).
    pub async fn remove(
        &self,
        provider: CloudProvider,
        context: &ContextName,
    ) -> OxiResult<RemovedCluster> {
        let _write = self.inner.write.lock().await;
        let mut file = self.inner.managed.read(provider).await?;
        if !file.remove(context) {
            return Err(OxiError::not_found(format!(
                "{context} is not among the added {provider} clusters."
            )));
        }
        let path = self.managed_path(provider);
        // Before the reload drops it from the catalog.
        let session = self.session_of(&path, context).await;
        let deleted_file = file.is_empty();
        if deleted_file {
            self.forget_file(&path).await?;
        } else {
            self.inner.managed.write(provider, &file).await?;
            self.inner.sources.reload().await?;
        }
        let disconnected = session.is_some_and(|cluster| self.disconnect(&cluster));
        Ok(RemovedCluster {
            context: context.clone(),
            path,
            deleted_file,
            disconnected,
        })
    }

    fn is_disabled(&self, provider: CloudProvider) -> bool {
        self.inner.config.lock().disabled.contains(&provider)
    }

    fn port(&self, provider: CloudProvider) -> Option<Arc<dyn CloudDiscoveryPort>> {
        self.inner
            .providers
            .iter()
            .find(|port| port.id() == provider)
            .cloned()
    }

    /// Deletes a managed file that is now empty, with its source entry (the sources service
    /// deletes files it stores in the kubeconfigs directory).
    async fn forget_file(&self, path: &Path) -> OxiResult<()> {
        let listed = KubeconfigSourceRef::File {
            path: path_text(path),
        };
        match self.inner.sources.remove(&listed).await {
            Ok(_) => Ok(()),
            // Not listed (the user removed the source): delete the file alone.
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.inner.managed.delete(path).await?;
                self.inner.sources.reload().await.map(drop)
            }
            Err(error) => Err(error),
        }
    }

    /// The catalog id of `context` as read from the managed file at `path`, if it is loaded.
    async fn session_of(&self, path: &Path, context: &ContextName) -> Option<ClusterId> {
        let (sources, contexts) =
            futures::join!(self.inner.source.sources(), self.inner.source.contexts());
        let (Ok(sources), Ok(contexts)) = (sources, contexts) else {
            return None;
        };
        let source = sources.iter().find(|s| s.path.as_deref() == Some(path))?;
        contexts
            .into_iter()
            .find(|c| c.context == *context && c.source == source.id)
            .map(|c| c.cluster)
    }

    /// Disconnects `cluster` when it has an open session. Returns whether it had one.
    fn disconnect(&self, cluster: &ClusterId) -> bool {
        if self.inner.sessions.get(cluster).is_none() {
            return false;
        }
        match self.inner.sessions.disconnect(cluster) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, %cluster, "could not disconnect a removed cloud cluster");
                false
            }
        }
    }
}

fn no_adapter(provider: CloudProvider) -> OxiError {
    OxiError::unsupported(format!(
        "{provider} discovery is not available in this build."
    ))
}

/// A path as the source list stores it.
fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
