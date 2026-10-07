//! [`ClusterPrefs`]: what the user's settings say about one cluster, as plain values.
//!
//! The settings layer (`oxikube_settings::ClusterSettings`, E06-S08) resolves
//! `default.json`, the user's `settings.json` and the cluster's `clusters.<id>` block into a
//! [`ClusterPrefs`]. The app crate has no `gpui` and cannot read the settings store, so the
//! binary pushes a [`ClusterPrefsTable`] (the resolved prefs of every cluster that has
//! overrides, plus the global fallback) into `oxikube_app::ClusterSessionManager` at startup
//! and on every change. Everything here is a value type: no I/O, no secrets. A Prometheus
//! bearer token, for example, is only a [`SecretKey`] naming a keychain entry.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::ClusterColour;
use oxikube_domain::ids::ClusterId;

use crate::connector::ExecInteractivity;
use crate::exec::NodeShellToleration;
use crate::secrets::SecretKey;

/// Where a cluster's Prometheus is, when the user overrides auto-detection (E13).
///
/// Every field is optional so a layer can set any subset; the settings layer fills them from
/// `clusters.<id>.prometheus`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrometheusOverride {
    /// Provider id (`operator`, `victoria_metrics`, ...) or `auto`; `None` means auto-detect.
    pub provider: Option<String>,
    /// `namespace/service:port` reached through the API server's service proxy.
    pub path: Option<String>,
    /// A direct `http(s)` URL (no credentials, query or fragment).
    pub url: Option<String>,
    /// The keychain entry holding the bearer token. Only the name is ever configured.
    pub auth: Option<SecretKey>,
}

/// The node shell's pod template as the settings give it (E09-S09): the keys under
/// `node_shell` beside the flat `node_shell_image` and `node_shell_pull_secret`. Every field is
/// optional; [`NodeShellSpec::for_node`](crate::NodeShellSpec::for_node) puts what is set over
/// the defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeShellPrefs {
    /// Namespace of the helper pod.
    pub namespace: Option<String>,
    /// The command run in the node's namespaces; empty runs `bash -l` if the node has bash, else
    /// `sh -l`.
    pub command: Vec<String>,
    /// The `nsenter` options before the `--`; empty enters every namespace of the node's init.
    pub nsenter_args: Vec<String>,
    /// What the helper pod tolerates; `None` tolerates every taint, an empty list none.
    pub tolerations: Option<Vec<NodeShellToleration>>,
    /// Labels added to the helper pod.
    pub labels: BTreeMap<String, String>,
    /// `Always`, `IfNotPresent` or `Never`; `None` leaves the cluster's default.
    pub image_pull_policy: Option<String>,
    /// The longest the helper pod lives (`activeDeadlineSeconds`); `None` is eight hours.
    pub max_lifetime_seconds: Option<u64>,
}

/// The watch budget of a cluster (`watch_budget`, E04-F543): how many feeds and objects the
/// app may hold open on it, and how long an unobserved feed lingers.
///
/// The binary applies it to the kube adapter's per-connection `FeedRegistry` (every feed the
/// app opens goes through it) and to the resource store's idle teardown; a change applies to
/// the next feed opened and the next feed to go idle, without reconnecting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchBudgetPrefs {
    /// Most feeds open at once on the cluster (each namespace of a namespace set is one).
    pub max_feeds: usize,
    /// No new feed opens while the open feeds hold this many objects.
    pub max_objects: u64,
    /// While the open feeds hold this many objects, a kind that would get full objects gets a
    /// metadata-only feed instead.
    pub metadata_above: u64,
    /// How long a feed nobody looks at keeps running, so switching back to a view is instant.
    pub idle_grace: Duration,
}

impl Default for WatchBudgetPrefs {
    /// The adapter's defaults: 64 feeds, 100 000 objects, metadata-only above 25 000, 30 s.
    fn default() -> Self {
        Self {
            max_feeds: 64,
            max_objects: 100_000,
            metadata_above: 25_000,
            idle_grace: Duration::from_secs(30),
        }
    }
}

/// The per-cluster settings, resolved: defaults, then the user's global values, then
/// `clusters.<id>`, field by field.
///
/// `read_only`, `colour`, `display_name` and `exec_interactivity` are applied to the live
/// `ClusterSession` (the first three immediately, the exec policy on the next connect); the
/// rest are read by their features (terminal, node shell, Prometheus, namespace selector)
/// through `ClusterSession::prefs` on the session snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClusterPrefs {
    /// Human-readable name shown instead of the context name.
    pub display_name: Option<String>,
    /// Accent colour (hotbar dot, tab stripe, badges).
    pub colour: Option<ClusterColour>,
    /// Whether mutations are blocked (enforced by `MutationGuard`).
    pub read_only: bool,
    /// The namespace a new session starts in, over the kubeconfig context's own.
    pub default_namespace: Option<String>,
    /// Working directory of terminals opened for this cluster.
    pub terminal_cwd: Option<String>,
    /// Image of the node shell pod.
    pub node_shell_image: Option<String>,
    /// Name of the `imagePullSecret` the node shell pod references (a Kubernetes Secret name,
    /// not a credential).
    pub node_shell_pull_secret: Option<String>,
    /// The rest of the node shell's pod template (namespace, command, tolerations, ...).
    pub node_shell: NodeShellPrefs,
    /// Manual Prometheus location, if any.
    pub prometheus: Option<PrometheusOverride>,
    /// Namespaces to offer when the user may not list namespaces cluster-wide.
    pub accessible_namespaces: Vec<String>,
    /// How interactive exec credential plugins may be; read at connect time.
    pub exec_interactivity: ExecInteractivity,
    /// Whether a shell, attach or exec into a pod (`pod::Shell`, `pod::Attach`, `pod::Exec`) is
    /// allowed while `read_only` is on. `false` (the default) blocks them, since a shell can
    /// change anything the container's user can. Read by the guard on every open, so a change
    /// applies at once.
    pub exec_in_read_only: bool,
    /// Limits on the feeds the app opens on the cluster.
    pub watch_budget: WatchBudgetPrefs,
}

/// The resolved prefs of every cluster: a global fallback and a lookup index of the clusters
/// that have overrides of their own, so reading one cluster's prefs is one hash lookup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClusterPrefsTable {
    default: Arc<ClusterPrefs>,
    clusters: HashMap<ClusterId, Arc<ClusterPrefs>>,
}

impl ClusterPrefsTable {
    /// A table whose clusters all read `default` until [`with_cluster`](Self::with_cluster)
    /// adds overrides.
    pub fn new(default: impl Into<Arc<ClusterPrefs>>) -> Self {
        Self {
            default: default.into(),
            clusters: HashMap::new(),
        }
    }

    /// Adds the resolved prefs of a cluster that has `clusters.<id>` overrides.
    #[must_use]
    pub fn with_cluster(mut self, cluster: ClusterId, prefs: impl Into<Arc<ClusterPrefs>>) -> Self {
        self.clusters.insert(cluster, prefs.into());
        self
    }

    /// The prefs of `cluster`: its own when it has overrides, else the global fallback.
    pub fn get(&self, cluster: &ClusterId) -> &Arc<ClusterPrefs> {
        self.clusters.get(cluster).unwrap_or(&self.default)
    }

    /// How many clusters have prefs of their own.
    pub fn len(&self) -> usize {
        self.clusters.len()
    }

    /// Whether no cluster has prefs of its own.
    pub fn is_empty(&self) -> bool {
        self.clusters.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ids::ContextName;

    use super::*;

    fn id(name: &str) -> ClusterId {
        ClusterId::new("kubeconfig", &ContextName::new(name))
    }

    #[test]
    fn a_cluster_without_overrides_reads_the_fallback() {
        let table = ClusterPrefsTable::new(ClusterPrefs {
            read_only: true,
            ..ClusterPrefs::default()
        })
        .with_cluster(id("prod"), ClusterPrefs::default());
        assert!(!table.get(&id("prod")).read_only);
        assert!(table.get(&id("lab")).read_only);
        assert_eq!(table.len(), 1);
        assert!(!table.is_empty());
    }
}
