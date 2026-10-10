//! What the jump bar reads from the running app: [`JumpSources`], the data [`Loaded`] off the UI
//! thread, and [`LiveEnv`], the snapshot a line is parsed and planned against.
//!
//! The alias table of a cluster is read live (a cheap handle). The cluster contexts (read from the
//! kubeconfig sources) and the namespaces of the shown cluster (a list call) are loaded on the
//! Tokio bridge when the bar opens and kept for the next open; a line typed before they arrive is
//! checked against what the previous open found, or not against them at all (a namespace is
//! accepted when the cluster's list is not known).

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::App;
use oxikube_app::search::aliases::{AliasRegistry, AliasTable};
use oxikube_app::search::jump::{JumpContext, JumpEnv};
use oxikube_app::session::namespaces::{NamespaceCatalog, NamespaceService};
use oxikube_app::{ClusterCatalog, ClusterSessionManager};
use oxikube_domain::ids::ClusterId;

/// The live app state the bar needs. Cheap to clone.
#[derive(Clone)]
pub struct JumpSources {
    /// The cluster whose tab is shown, read when the bar opens.
    pub active: Rc<dyn Fn(&App) -> Option<ClusterId>>,
    /// The sessions: which contexts are connected, and what waits for a connection.
    pub sessions: ClusterSessionManager,
    /// The cluster contexts the user can switch to.
    pub catalog: ClusterCatalog,
    /// The alias table of every cluster.
    pub aliases: AliasRegistry,
    /// The namespace lists.
    pub namespaces: NamespaceService,
}

/// Contexts and namespaces read off the UI thread; what the bar knows between opens.
#[derive(Debug, Clone, Default)]
pub struct Loaded {
    /// The contexts in catalog order: name and cluster.
    pub contexts: Vec<(Arc<str>, ClusterId)>,
    /// The namespace list of each cluster that was asked.
    pub namespaces: HashMap<ClusterId, NamespaceCatalog>,
}

impl Loaded {
    /// Reads the catalog and, for `active`, the namespaces. Runs on the Tokio bridge: it lists
    /// kubeconfig files and calls the cluster. A failed read leaves that part empty.
    pub async fn read(
        catalog: ClusterCatalog,
        namespaces: NamespaceService,
        active: Option<ClusterId>,
    ) -> Loaded {
        let contexts = async {
            match catalog.load().await {
                Ok(mut entries) => {
                    entries.sort_by(|a, b| a.cmp_default(b));
                    entries
                        .iter()
                        .map(|e| (Arc::<str>::from(e.name()), e.id().clone()))
                        .collect()
                }
                Err(error) => {
                    tracing::warn!(%error, "the jump bar could not list the cluster contexts");
                    Vec::new()
                }
            }
        };
        let names = async {
            let mut found = HashMap::new();
            if let Some(cluster) = active {
                match namespaces.catalog(&cluster).await {
                    Ok(list) => {
                        found.insert(cluster, list);
                    }
                    Err(error) => {
                        tracing::debug!(%error, "the jump bar could not list the namespaces");
                    }
                }
            }
            found
        };
        let (contexts, namespaces) = futures::join!(contexts, names);
        Loaded {
            contexts,
            namespaces,
        }
    }

    /// Takes what `newer` found: its contexts, and its namespace lists over the older ones.
    pub fn merge(&mut self, newer: Loaded) {
        if !newer.contexts.is_empty() {
            self.contexts = newer.contexts;
        }
        self.namespaces.extend(newer.namespaces);
    }
}

/// The snapshot a line is resolved against ([`JumpEnv`]): taken when the bar opens and again when
/// the data it was waiting for arrives.
pub struct LiveEnv {
    active: Option<ClusterId>,
    contexts: Vec<JumpContext>,
    aliases: AliasRegistry,
    namespaces: HashMap<ClusterId, NamespaceCatalog>,
}

impl LiveEnv {
    /// The state of the app now, with what was `loaded`.
    pub fn snapshot(sources: &JumpSources, loaded: &Loaded, cx: &App) -> Self {
        let contexts = loaded
            .contexts
            .iter()
            .map(|(name, cluster)| JumpContext {
                name: name.clone(),
                cluster: cluster.clone(),
                connected: sources
                    .sessions
                    .get(cluster)
                    .is_some_and(|session| session.is_connected()),
            })
            .collect();
        Self {
            active: (sources.active)(cx),
            contexts,
            aliases: sources.aliases.clone(),
            namespaces: loaded.namespaces.clone(),
        }
    }
}

impl JumpEnv for LiveEnv {
    fn active_cluster(&self) -> Option<ClusterId> {
        self.active.clone()
    }

    fn contexts(&self) -> &[JumpContext] {
        &self.contexts
    }

    fn aliases(&self, cluster: &ClusterId) -> AliasTable {
        self.aliases.table(cluster)
    }

    fn namespaces(&self, cluster: &ClusterId) -> Option<NamespaceCatalog> {
        self.namespaces.get(cluster).cloned()
    }
}
