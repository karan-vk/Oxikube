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
}

impl Diagnostic {
    /// How loudly to show this diagnostic.
    pub fn severity(&self) -> Severity {
        match self {
            Diagnostic::MissingFile { .. } | Diagnostic::BlankFile { .. } => Severity::Info,
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
