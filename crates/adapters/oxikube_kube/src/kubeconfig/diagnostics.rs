//! What happened to each kubeconfig file, reported instead of failing the load.
//!
//! Nothing here carries file contents: a [`Diagnostic`] names a file (and, for duplicates, a
//! context name) and says what was wrong in fixed wording. Parse failures deliberately drop the
//! parser's message, because it can quote the offending line, which may hold a token or key.

use std::fmt;
use std::path::PathBuf;

use oxikube_domain::ids::ContextName;

/// How loudly a [`Diagnostic`] should be shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Expected and harmless (a path that does not exist yet, an empty file).
    Info,
    /// The user probably wants to know (a broken file, a shadowed context).
    Warning,
}

/// Which input supplied the kubeconfig paths (E03-S10). Tiers are exclusive, in this order of
/// precedence, as in kubectl: the first one that yields any path is the only one loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceTier {
    /// Paths passed in explicitly (settings, a `--kubeconfig` flag).
    Explicit,
    /// The `KUBECONFIG` environment variable.
    KubeconfigEnv,
    /// `~/.kube/config`.
    DefaultPath,
}

impl fmt::Display for SourceTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SourceTier::Explicit => "explicit sources",
            SourceTier::KubeconfigEnv => "KUBECONFIG",
            SourceTier::DefaultPath => "the default kubeconfig path",
        })
    }
}

/// Why the in-cluster fallback did not run although no kubeconfig context was usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InClusterSkip {
    /// The process does not look like it runs in a pod.
    NotInCluster,
    /// A kubeconfig file exists but could not be read or parsed. Falling back would silently
    /// change which cluster the app talks to, so the diagnostics are reported instead.
    BrokenKubeconfig,
}

/// One thing worth telling the user about a load.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Diagnostic {
    /// The path does not exist. Skipped.
    MissingFile {
        /// The kubeconfig path as listed.
        path: PathBuf,
    },
    /// The file holds no clusters, users, contexts or current-context. Skipped.
    BlankFile {
        /// The kubeconfig path as listed.
        path: PathBuf,
    },
    /// The file exists but could not be read (permissions, a directory, not UTF-8). Skipped.
    Unreadable {
        /// The kubeconfig path as listed.
        path: PathBuf,
        /// The I/O error kind, e.g. "permission denied". Never file contents.
        reason: String,
    },
    /// The file was read but is not a valid kubeconfig. Skipped; contents are not reported.
    Unparsable {
        /// The kubeconfig path as listed.
        path: PathBuf,
    },
    /// The file's `kind` or `apiVersion` conflicts with an earlier file. Skipped.
    Incompatible {
        /// The kubeconfig path as listed.
        path: PathBuf,
        /// Why the merge was refused (fixed wording from kube).
        reason: String,
    },
    /// Two kubeconfig entries define the same context name. The first file wins (kubectl's
    /// rule); `shadowed` is ignored for this context. When both are the same path, the file
    /// defines the name twice.
    DuplicateContext {
        /// The duplicated context name.
        context: ContextName,
        /// The file whose definition is used.
        winner: PathBuf,
        /// The file whose definition is ignored.
        shadowed: PathBuf,
    },
    /// A directory source could not be registered with the file watcher, so changes in it are
    /// only noticed by the periodic poll (E03-F439). Reported by the sources adapter, not by
    /// the loader.
    DirectoryNotWatched {
        /// The directory as listed.
        path: PathBuf,
    },
    /// Which input supplied the paths that were loaded (E03-S10).
    SourceSelected {
        /// The winning tier.
        tier: SourceTier,
        /// How many paths it listed.
        paths: usize,
    },
    /// No explicit source, `KUBECONFIG` entry or home directory gave any path to load.
    NoKubeconfigSource,
    /// No kubeconfig context was usable and the in-cluster service account was used instead.
    InClusterUsed,
    /// No kubeconfig context was usable and the in-cluster fallback was not used.
    InClusterSkipped {
        /// Why.
        reason: InClusterSkip,
    },
}

impl Diagnostic {
    /// How loudly to show this diagnostic.
    pub fn severity(&self) -> Severity {
        match self {
            Diagnostic::MissingFile { .. }
            | Diagnostic::BlankFile { .. }
            | Diagnostic::SourceSelected { .. }
            | Diagnostic::NoKubeconfigSource
            | Diagnostic::InClusterUsed
            | Diagnostic::InClusterSkipped {
                reason: InClusterSkip::NotInCluster,
            } => Severity::Info,
            _ => Severity::Warning,
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Diagnostic::MissingFile { path } => {
                write!(f, "kubeconfig {} does not exist; skipped", path.display())
            }
            Diagnostic::BlankFile { path } => {
                write!(f, "kubeconfig {} is blank; skipped", path.display())
            }
            Diagnostic::Unreadable { path, reason } => {
                write!(
                    f,
                    "kubeconfig {} could not be read ({reason}); skipped",
                    path.display()
                )
            }
            Diagnostic::Unparsable { path } => write!(
                f,
                "kubeconfig {} is not a valid kubeconfig; skipped",
                path.display()
            ),
            Diagnostic::Incompatible { path, reason } => {
                write!(
                    f,
                    "kubeconfig {} cannot be merged ({reason}); skipped",
                    path.display()
                )
            }
            Diagnostic::DirectoryNotWatched { path } => write!(
                f,
                "directory {} could not be watched for changes; it is checked at every poll interval",
                path.display()
            ),
            Diagnostic::SourceSelected { tier, paths } => {
                write!(f, "kubeconfig source: {tier} ({paths} path(s))")
            }
            Diagnostic::NoKubeconfigSource => f.write_str("no kubeconfig path to load"),
            Diagnostic::InClusterUsed => {
                f.write_str("no kubeconfig context found; using the in-cluster service account")
            }
            Diagnostic::InClusterSkipped {
                reason: InClusterSkip::NotInCluster,
            } => f.write_str("no kubeconfig context found and not running in a cluster"),
            Diagnostic::InClusterSkipped {
                reason: InClusterSkip::BrokenKubeconfig,
            } => f.write_str(
                "a kubeconfig file could not be loaded; not falling back to the in-cluster \
                 service account",
            ),
            Diagnostic::DuplicateContext {
                context,
                winner,
                shadowed,
            } if winner == shadowed => write!(
                f,
                "context {context:?} is defined more than once in {}; the first definition is used",
                winner.display()
            ),
            Diagnostic::DuplicateContext {
                context,
                winner,
                shadowed,
            } => write!(
                f,
                "context {context:?} is defined in {} and {}; {} wins",
                winner.display(),
                shadowed.display(),
                winner.display()
            ),
        }
    }
}

/// Outcome of reading one listed kubeconfig path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceStatus {
    /// Parsed and merged.
    Loaded,
    /// The path does not exist.
    Missing,
    /// Parsed but empty.
    Blank,
    /// Could not be read.
    Unreadable,
    /// Could not be parsed.
    Unparsable,
    /// Parsed but refused by the merge (`kind` / `apiVersion` mismatch).
    Incompatible,
}

/// One consulted kubeconfig path, in `KUBECONFIG` order.
///
/// Every listed path appears here, usable or not, so a watcher (E03-S02) can observe files that
/// do not exist yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInfo {
    /// The path as listed.
    pub path: PathBuf,
    /// Stable identity of the file for [`ClusterId`](oxikube_domain::ids::ClusterId)
    /// derivation: the canonical path when it resolves, otherwise `path` as given.
    pub key: String,
    /// What happened to it.
    pub status: SourceStatus,
    /// Every context name the file defines, including ones shadowed by an earlier file. Empty
    /// unless `status` is [`SourceStatus::Loaded`].
    pub contexts: Vec<ContextName>,
}

impl SourceInfo {
    /// True for the synthetic source of the in-cluster context (not a file on disk).
    pub fn is_in_cluster(&self) -> bool {
        self.path == std::path::Path::new(super::incluster::IN_CLUSTER_SOURCE_PATH)
    }
}
