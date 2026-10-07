//! [`ClusterSourcePort`]: where cluster contexts come from (kubeconfig files and more).
//!
//! # Adapter
//!
//! Implemented by `oxikube_kube::sources` (E03-S02 defines the behaviour): kubeconfig
//! files and directories, `KUBECONFIG`, in-cluster config and cloud-imported entries.
//! The port lists sources and their contexts, streams [`SourcesChanged`] diffs when a
//! file changes, and can be told to [`reload`](ClusterSourcePort::reload). What the last read
//! skipped or shadowed is available as [`SourceDiagnostic`]s
//! ([`source_diagnostics`](ClusterSourcePort::source_diagnostics)), with a stream that fires
//! when that list changes.
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

/// What kind of entry the user's source list holds (the settings key `kubeconfig.sources`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserSourceKind {
    /// What kubectl reads: the files named by `KUBECONFIG`, else `~/.kube/config`. Carries no
    /// path.
    Default,
    /// One kubeconfig file.
    File,
    /// A directory of kubeconfig files (not recursive).
    Dir,
}

/// One entry of the user's source list: the part of the catalog the user manages (E06-S05).
///
/// The list lives in settings (`kubeconfig.sources`); the adapter is told about it with
/// [`ClusterSourcePort::set_user_sources`] and reads exactly those sources.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserSource {
    /// What it is.
    pub kind: UserSourceKind,
    /// The file or directory; `None` for [`UserSourceKind::Default`].
    pub path: Option<PathBuf>,
}

impl UserSource {
    /// The `KUBECONFIG` / `~/.kube/config` entry.
    pub fn default_source() -> Self {
        Self {
            kind: UserSourceKind::Default,
            path: None,
        }
    }

    /// A single kubeconfig file.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: UserSourceKind::File,
            path: Some(path.into()),
        }
    }

    /// A directory of kubeconfig files.
    pub fn dir(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: UserSourceKind::Dir,
            path: Some(path.into()),
        }
    }
}

/// How reading one source went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceState {
    /// Read; its contexts are in the catalog (the count may be zero for an empty directory).
    Found,
    /// The file is blank: no clusters, users or contexts.
    Blank,
    /// The path does not exist.
    Missing,
    /// The path exists but could not be read (permissions, not a file, not text).
    Unreadable,
    /// Read, but not a valid kubeconfig (or one that cannot be merged with the others).
    Invalid,
}

/// The outcome of the last read of one source, for the sources screen.
///
/// Carries no file content: `message` is fixed wording that names files, never their text
/// (parser messages can quote a token, so they are dropped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStatus {
    /// The source this describes.
    pub source: ClusterSource,
    /// How the read went.
    pub state: SourceState,
    /// How many contexts the source contributed (after first-file-wins deduplication it may be
    /// fewer than the file defines).
    pub contexts: usize,
    /// Why the source is not fully usable, or a note about it. `None` when all is well.
    pub message: Option<String>,
}

/// How loudly a [`SourceDiagnostic`] should be shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticSeverity {
    /// Expected and harmless: a path that does not exist yet, a blank file, which input was
    /// chosen.
    Info,
    /// The user probably wants to know: a file that could not be read or parsed, a shadowed
    /// context, a directory that is not watched.
    Warning,
}

/// One thing worth telling the user about the last read of the sources ("file X could not be
/// read"), for the sources screen.
///
/// This is the port-level view of the adapter's richer loader diagnostic: the adapter keeps its
/// own enum and maps it here, so the port does not depend on the loader's shape. Like
/// [`SourceStatus`] it carries no file content. `message` is fixed wording that names files
/// and context names, never their text (parser messages can quote a token, so they are
/// dropped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDiagnostic {
    /// How loudly to show it.
    pub severity: DiagnosticSeverity,
    /// The file or directory it is about, when it is about one. Matches
    /// [`ClusterSource::path`] for file and directory sources, or a file inside a directory
    /// source. `None` for diagnostics about the load as a whole (which input was chosen, the
    /// in-cluster fallback) and for pasted kubeconfigs, which are not files.
    pub path: Option<PathBuf>,
    /// Plain text for display. No secrets.
    pub message: String,
}

impl SourceDiagnostic {
    /// A diagnostic with no path.
    pub fn new(severity: DiagnosticSeverity, message: impl Into<String>) -> Self {
        Self {
            severity,
            path: None,
            message: message.into(),
        }
    }

    /// Names the file or directory the diagnostic is about.
    #[must_use]
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }
}

impl std::fmt::Display for SourceDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
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
    /// The name of the kubeconfig `cluster` entry the context points at, if it names one.
    /// A name only, never the server's credentials.
    pub cluster_name: Option<String>,
    /// The name of the kubeconfig `user` entry the context points at, if it names one. A name
    /// only: tokens, certificates and exec plugins stay inside the adapter.
    pub user: Option<String>,
    /// Why the context cannot work as written (it names a cluster or user the kubeconfig does
    /// not define), or `None` when it looks usable. The catalog keeps such entries and flags
    /// them instead of hiding them; connecting will fail with the adapter's own error. Plain
    /// text for display: no secrets.
    pub problem: Option<String>,
}

impl ClusterContext {
    /// A context with just its identity and source: no server, namespace, cluster or user
    /// names, and no problem. Fill the rest in with struct update syntax.
    pub fn new(cluster: ClusterId, context: ContextName, source: SourceId) -> Self {
        Self {
            cluster,
            context,
            source,
            server: None,
            default_namespace: None,
            cluster_name: None,
            user: None,
            problem: None,
        }
    }
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
///
/// # Effects
///
/// Read-only: it reads kubeconfig sources and never writes them.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// [`NotFound`](oxikube_domain::ErrorKind::NotFound) for a missing source path,
/// [`Validation`](oxikube_domain::ErrorKind::Validation) for a kubeconfig that does not parse,
/// [`Internal`](oxikube_domain::ErrorKind::Internal) for other I/O failures.
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

    /// Replaces the user's source list (settings `kubeconfig.sources`) and re-reads. The
    /// [`Default`](UserSourceKind::Default) entry stands for `KUBECONFIG` / `~/.kube/config`;
    /// without it those are not read. Entries naming the same path count once. Returns what
    /// changed, as [`reload`](Self::reload) does.
    ///
    /// A path that is missing or not a kubeconfig is not an error here: it is listed with its
    /// problem in [`source_statuses`](Self::source_statuses) and the other sources still load.
    async fn set_user_sources(&self, sources: &[UserSource]) -> OxiResult<SourcesChanged>;

    /// How the last read of each source went, in source order. Local: no cluster is contacted.
    async fn source_statuses(&self) -> OxiResult<Vec<SourceStatus>>;

    /// What the last read skipped, shadowed or could not watch, in the order the adapter found
    /// it. Local: no cluster is contacted. Empty when all is well, and before the first read
    /// (the first call reads the sources, as [`contexts`](Self::contexts) does).
    ///
    /// Complements [`source_statuses`](Self::source_statuses): that is one row per source, this
    /// is the list of individual findings (a shadowed context, a directory that could not be
    /// watched, why the in-cluster fallback did not run).
    async fn source_diagnostics(&self) -> OxiResult<Vec<SourceDiagnostic>>;

    /// A stream of the full diagnostics list each time it changes, from now on. A reload that
    /// leaves the list as it was emits nothing, and nothing is replayed on subscribe (call
    /// [`source_diagnostics`](Self::source_diagnostics) for the current list). Changes that do
    /// not alter the catalog, such as a new broken file, are reported here and not on
    /// [`subscribe`](Self::subscribe).
    fn subscribe_diagnostics(&self) -> BoxStream<'static, Vec<SourceDiagnostic>>;

    /// Checks that `text` is a usable kubeconfig, without storing it, merging it or touching
    /// the network, and returns how many contexts it defines.
    ///
    /// # Errors
    ///
    /// [`Validation`](oxikube_domain::ErrorKind::Validation) for text that is empty or not a
    /// kubeconfig. The message is fixed wording and never quotes the text, which may hold
    /// credentials.
    async fn validate_kubeconfig(&self, text: &str) -> OxiResult<usize>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(name: &str, server: &str) -> ClusterContext {
        let context = ContextName::new(name);
        ClusterContext {
            server: Some(server.into()),
            ..ClusterContext::new(
                ClusterId::new("/kube/config", &context),
                context,
                SourceId("file".into()),
            )
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
    fn a_diagnostic_displays_as_its_message_and_may_name_a_path() {
        let plain = SourceDiagnostic::new(DiagnosticSeverity::Info, "no kubeconfig path to load");
        assert_eq!(plain.path, None);
        assert_eq!(plain.to_string(), "no kubeconfig path to load");
        let named = SourceDiagnostic::new(DiagnosticSeverity::Warning, "x").with_path("/a.yaml");
        assert_eq!(named.path.as_deref(), Some(std::path::Path::new("/a.yaml")));
        assert!(DiagnosticSeverity::Info < DiagnosticSeverity::Warning);
    }

    #[test]
    fn identical_snapshots_diff_to_empty() {
        let snapshot = [ctx("a", "https://a")];
        assert!(SourcesChanged::diff(&snapshot, &snapshot).is_empty());
    }
}
