//! [`AliasFollow`]: keeps the registry's tables in step with the sessions' API discovery.
//!
//! One task reads the session updates and hands each cluster's work to that cluster's own
//! worker, so a cluster whose discovery is slow delays no other. The worker asks the cluster's
//! `DiscoveryPort`, which answers from the adapter's cache after the connect, and rebuilds the
//! table on the runtime; the UI thread only ever reads.
//!
//! | Session change | Work |
//! |---|---|
//! | the session becomes connected | list every served kind, replace the discovery layer |
//! | `KindsChanged` with details | resolve the added, changed and removed kinds, apply only those |
//! | `KindsChanged` without details (the subscriber fell behind) | list every served kind again |
//! | the session stops being connected | clear the discovery layer (built-in and user aliases stay) |
//! | the session is closed | drop the worker and the table |

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::kinds::ResourceKind;
use oxikube_ports::{DiscoveryPort, KindsChange};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use super::registry::AliasRegistry;
use super::table::AliasTable;
use crate::session::{ClusterSessionManager, SessionChange, SessionUpdate};

/// The follower task. Aborts it when dropped, and with it every worker.
#[derive(Debug)]
pub struct AliasFollow(JoinHandle<()>);

impl Drop for AliasFollow {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) fn start(
    registry: AliasRegistry,
    sessions: ClusterSessionManager,
    runtime: Handle,
) -> AliasFollow {
    let follower = follower(registry, sessions, runtime.clone());
    AliasFollow(runtime.spawn(follower))
}

/// The follower loop as a plain future: it reads the session updates and hands the work to the
/// per-cluster workers (which run on `runtime`, and only exist once a cluster has a session).
/// Dropping the future aborts every worker. The app runs it as a GPUI task so an idle app holds
/// no task of its own on the Tokio runtime.
pub(super) fn follower(
    registry: AliasRegistry,
    sessions: ClusterSessionManager,
    runtime: Handle,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    // Subscribe before reading the sessions, so no update falls between the two.
    let mut updates = sessions.subscribe();
    async move {
        let mut workers = Workers::new(registry, runtime);
        workers.resync(&sessions);
        while let Some(item) = updates.next().await {
            match item {
                Ok(update) => workers.on_update(&sessions, update),
                // Missed some: look at the sessions as they are now.
                Err(_) => workers.resync(&sessions),
            }
        }
    }
}

/// What a cluster's worker does next.
enum Job {
    /// List every served kind.
    Reload(Arc<dyn DiscoveryPort>),
    /// Apply one CRD change.
    Apply(Arc<dyn DiscoveryPort>, KindsChange),
    /// Forget the discovery layer.
    Clear,
}

struct Worker {
    jobs: mpsc::UnboundedSender<Job>,
    task: JoinHandle<()>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Workers {
    registry: AliasRegistry,
    runtime: Handle,
    by_cluster: HashMap<ClusterId, Worker>,
}

impl Workers {
    fn new(registry: AliasRegistry, runtime: Handle) -> Self {
        Self {
            registry,
            runtime,
            by_cluster: HashMap::new(),
        }
    }

    fn send(&mut self, cluster: &ClusterId, job: Job) {
        let worker = self.by_cluster.entry(cluster.clone()).or_insert_with(|| {
            let (jobs, rx) = mpsc::unbounded();
            let table = self.registry.table(cluster);
            Worker {
                jobs,
                task: self.runtime.spawn(run(cluster.clone(), table, rx)),
            }
        });
        let _ = worker.jobs.unbounded_send(job);
    }

    /// Brings every cluster's table in line with its session as it is now.
    fn resync(&mut self, sessions: &ClusterSessionManager) {
        for session in sessions.sessions() {
            let cluster = session.id().clone();
            match session.discovery().filter(|_| session.is_connected()) {
                Some(discovery) => self.send(&cluster, Job::Reload(discovery)),
                None => self.send(&cluster, Job::Clear),
            }
        }
    }

    fn on_update(&mut self, sessions: &ClusterSessionManager, update: SessionUpdate) {
        let SessionUpdate { cluster, change } = update;
        match change {
            SessionChange::Closed => {
                self.by_cluster.remove(&cluster);
                self.registry.forget(&cluster);
            }
            SessionChange::StateChanged { from, state } => {
                let connected = state.phase().is_connected();
                if connected && !from.is_connected() {
                    if let Some(discovery) = sessions.get(&cluster).and_then(|s| s.discovery()) {
                        self.send(&cluster, Job::Reload(discovery));
                    }
                } else if !connected && from.is_connected() {
                    self.send(&cluster, Job::Clear);
                }
            }
            SessionChange::KindsChanged(change) => {
                let Some(discovery) = sessions.get(&cluster).and_then(|s| s.discovery()) else {
                    return;
                };
                let no_details = change.added.is_empty()
                    && change.removed.is_empty()
                    && change.changed.is_empty();
                if no_details {
                    self.send(&cluster, Job::Reload(discovery));
                } else {
                    self.send(&cluster, Job::Apply(discovery, change));
                }
            }
            _ => {}
        }
    }
}

/// One cluster's worker: runs its jobs in order.
async fn run(cluster: ClusterId, table: AliasTable, mut jobs: mpsc::UnboundedReceiver<Job>) {
    while let Some(job) = jobs.next().await {
        match job {
            Job::Clear => table.clear_discovered(),
            Job::Reload(discovery) => reload(&cluster, &table, discovery.as_ref()).await,
            Job::Apply(discovery, change) => {
                apply(&cluster, &table, discovery.as_ref(), &change).await;
            }
        }
    }
}

async fn reload(cluster: &ClusterId, table: &AliasTable, discovery: &dyn DiscoveryPort) {
    match discovery.discover().await {
        Ok(kinds) => table.set_discovered(&kinds),
        Err(error) => {
            tracing::warn!(%error, %cluster, "aliases: listing the served kinds failed");
        }
    }
}

/// Applies a CRD change by asking about exactly the kinds it names.
///
/// Discovery reports a removal per served version, so a removed version that no longer resolves
/// may leave the type served at another one (`v1beta1` dropped, `v1` stays). Such a type is asked
/// about again without a version and kept; only a type the server no longer serves at all goes.
async fn apply(
    cluster: &ClusterId,
    table: &AliasTable,
    discovery: &dyn DiscoveryPort,
    change: &KindsChange,
) {
    let mut upserted = Vec::new();
    let mut gone: Vec<Gvk> = Vec::new();
    let named = change.added.iter().chain(&change.changed);
    for (gvk, removed) in named
        .map(|gvk| (gvk, false))
        .chain(change.removed.iter().map(|gvk| (gvk, true)))
    {
        match resolve_named(discovery, gvk, removed).await {
            Ok(Some(kind)) => upserted.push(kind),
            Ok(None) => gone.push(gvk.clone()),
            Err(error) => {
                // Do not guess: list everything again.
                tracing::warn!(%error, %cluster, kind = %gvk, "aliases: resolving a changed kind failed");
                reload(cluster, table, discovery).await;
                return;
            }
        }
    }
    table.apply_kinds_change(&gone, &upserted);
}

/// Resolves one kind a change names. A `removed` version that is gone falls back to the type's
/// preferred remaining version.
async fn resolve_named(
    discovery: &dyn DiscoveryPort,
    gvk: &Gvk,
    removed: bool,
) -> OxiResult<Option<ResourceKind>> {
    let found = discovery.resolve(gvk).await?;
    if found.is_some() || !removed || gvk.version.is_empty() {
        return Ok(found);
    }
    discovery
        .resolve(&Gvk::new(&*gvk.group, "", &*gvk.kind))
        .await
}
