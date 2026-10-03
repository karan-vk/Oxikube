//! [`ClusterSourcePort`]: where cluster contexts come from (kubeconfig files and more).
//!
//! # Adapter
//!
//! Implemented by `oxikube_kube::sources` (E03-S02 defines the behaviour): kubeconfig
//! files and directories, `KUBECONFIG`, in-cluster config and cloud-imported entries.
//! The port lists sources and their contexts, streams [`SourcesChanged`] diffs when a
//! file changes, and can be told to [`reload`](ClusterSourcePort::reload).
//!
//! Contexts are keyed by [`ClusterId`]. They carry no credentials: the adapter keeps
//! tokens and exec plugins to itself (non-negotiable 5).

use std::path::PathBuf;

use async_trait::async_trait;
use futures::stream::BoxStream;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::{ClusterId, ContextName};

/// Identifies a source of contexts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub String);

/// What kind of place a source is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    /// A single kubeconfig file.
    KubeconfigFile,
    /// A directory of kubeconfig files.
    KubeconfigDir,
    /// The `KUBECONFIG` environment variable.
    Environment,
    /// The pod's service account.
    InCluster,
    /// Clusters imported from a cloud provider.
    Cloud,
}

/// A place contexts are read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterSource {
    /// Stable identifier.
    pub id: SourceId,
    /// What it is.
    pub kind: SourceKind,
    /// Display label.
    pub label: String,
    /// Filesystem location, for file and directory sources.
    pub path: Option<PathBuf>,
}

/// One context of one source: an entry of the cluster catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterContext {
    /// Stable identity of the cluster entry.
    pub cluster: ClusterId,
    /// The context's name in its kubeconfig.
    pub context: ContextName,
    /// The source it came from.
    pub source: SourceId,
    /// API server URL, if known.
    pub server: Option<String>,
    /// The context's default namespace, if set.
    pub default_namespace: Option<String>,
}

/// The difference between two snapshots of the catalog, keyed by [`ClusterId`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourcesChanged {
    /// Contexts that appeared.
    pub added: Vec<ClusterContext>,
    /// Contexts that disappeared.
    pub removed: Vec<ClusterId>,
    /// Contexts still present whose details changed (new value).
    pub changed: Vec<ClusterContext>,
}

impl SourcesChanged {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// The diff from `old` to `new`. Order follows `new` for added and changed and
    /// `old` for removed.
    pub fn diff(old: &[ClusterContext], new: &[ClusterContext]) -> Self {
        let mut diff = Self::default();
        for ctx in new {
            match old.iter().find(|o| o.cluster == ctx.cluster) {
                None => diff.added.push(ctx.clone()),
                Some(previous) if previous != ctx => diff.changed.push(ctx.clone()),
                Some(_) => {}
            }
        }
        for ctx in old {
            if !new.iter().any(|n| n.cluster == ctx.cluster) {
                diff.removed.push(ctx.cluster.clone());
            }
        }
        diff
    }
}

/// Provides the cluster catalog.
#[async_trait]
pub trait ClusterSourcePort: Send + Sync {
    /// The configured sources.
    async fn sources(&self) -> OxiResult<Vec<ClusterSource>>;

    /// Every context of every source.
    async fn contexts(&self) -> OxiResult<Vec<ClusterContext>>;

    /// A stream of catalog changes from now on. Empty diffs are not emitted.
    fn subscribe(&self) -> BoxStream<'static, SourcesChanged>;

    /// Re-reads all sources now and returns what changed since the last read. The
    /// same diff is also sent to subscribers.
    async fn reload(&self) -> OxiResult<SourcesChanged>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(name: &str, server: &str) -> ClusterContext {
        let context = ContextName::new(name);
        ClusterContext {
            cluster: ClusterId::new("/kube/config", &context),
            context,
            source: SourceId("file".into()),
            server: Some(server.into()),
            default_namespace: None,
        }
    }

    #[test]
    fn diff_reports_added_removed_and_changed() {
        let old = [ctx("a", "https://a"), ctx("b", "https://b")];
        let new = [ctx("b", "https://b2"), ctx("c", "https://c")];
        let diff = SourcesChanged::diff(&old, &new);
        assert_eq!(diff.added, vec![ctx("c", "https://c")]);
        assert_eq!(diff.changed, vec![ctx("b", "https://b2")]);
        assert_eq!(diff.removed, vec![old[0].cluster.clone()]);
        assert!(!diff.is_empty());
    }

    #[test]
    fn identical_snapshots_diff_to_empty() {
        let snapshot = [ctx("a", "https://a")];
        assert!(SourcesChanged::diff(&snapshot, &snapshot).is_empty());
    }
}
