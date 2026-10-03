//! [`HelmPort`]: Helm releases of a cluster, read natively with CLI fallback.
//!
//! # Adapter
//!
//! Implemented by `oxikube_helm`: a native reader of the release `Secret`s
//! (`sh.helm.release.v1.*`) for listing and inspection, plus the `helm` CLI for the
//! operations that need it (research 2.5).
//!
//! # Mutating methods
//!
//! [`rollback`](HelmPort::rollback) and [`uninstall`](HelmPort::uninstall) are
//! **mutating**. They are reachable only through `MutationGuard` (read-only mode,
//! confirmation tier, dry-run where Helm supports it, audit); the UI never calls them
//! directly. All other methods are read-only.
//!
//! # Sensitivity
//!
//! Values and manifests can embed credentials. Callers show them but never persist
//! them (non-negotiable 5).

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;

/// Identifies a release: Helm release names are unique per namespace.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HelmReleaseRef {
    /// The cluster holding the release.
    pub cluster: ClusterId,
    /// The release's namespace.
    pub namespace: String,
    /// The release name.
    pub name: String,
}

/// A release's lifecycle state, as Helm reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HelmReleaseStatus {
    /// Helm does not know the state.
    Unknown,
    /// Installed and live.
    Deployed,
    /// Removed but history kept.
    Uninstalled,
    /// Replaced by a newer revision.
    Superseded,
    /// The last operation failed.
    Failed,
    /// An uninstall is in progress.
    Uninstalling,
    /// An install is in progress.
    PendingInstall,
    /// An upgrade is in progress.
    PendingUpgrade,
    /// A rollback is in progress.
    PendingRollback,
}

/// One revision of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelmRelease {
    /// Which release this revision belongs to.
    pub release: HelmReleaseRef,
    /// Revision number, starting at 1.
    pub revision: u32,
    /// State of this revision.
    pub status: HelmReleaseStatus,
    /// Chart name.
    pub chart: String,
    /// Chart version.
    pub chart_version: String,
    /// Version of the packaged application, if the chart declares one.
    pub app_version: Option<String>,
    /// When this revision was last updated.
    pub updated: Timestamp,
}

/// Helm access for a cluster.
///
/// # Effects
///
/// Read-only except [`rollback`](Self::rollback) and [`uninstall`](Self::uninstall), which are
/// **mutating** (see the module docs).
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// [`NotFound`](oxikube_domain::ErrorKind::NotFound) for an unknown release or revision,
/// [`Auth`](oxikube_domain::ErrorKind::Auth) /
/// [`Forbidden`](oxikube_domain::ErrorKind::Forbidden) when RBAC denies reading release
/// Secrets, [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) when the `helm` CLI is
/// needed and missing, [`Network`](oxikube_domain::ErrorKind::Network) /
/// [`Timeout`](oxikube_domain::ErrorKind::Timeout) (retryable) for connection failures.
#[async_trait]
pub trait HelmPort: Send + Sync {
    /// The latest revision of every release in the selected namespaces.
    async fn list_releases(
        &self,
        cluster: &ClusterId,
        namespaces: &NamespaceSelection,
    ) -> OxiResult<Vec<HelmRelease>>;

    /// Every revision of `release`, newest first.
    async fn history(&self, release: &HelmReleaseRef) -> OxiResult<Vec<HelmRelease>>;

    /// User-supplied values of a revision as YAML (latest revision when `None`).
    /// With `all`, includes chart defaults. May contain secrets; do not persist.
    async fn values(
        &self,
        release: &HelmReleaseRef,
        revision: Option<u32>,
        all: bool,
    ) -> OxiResult<String>;

    /// The rendered manifest of a revision (latest when `None`). May contain
    /// secrets; do not persist.
    async fn manifest(&self, release: &HelmReleaseRef, revision: Option<u32>) -> OxiResult<String>;

    /// **Mutating.** Rolls `release` back to `revision`. Only via `MutationGuard`.
    async fn rollback(&self, release: &HelmReleaseRef, revision: u32) -> OxiResult<()>;

    /// **Mutating.** Uninstalls `release`, keeping its history when `keep_history`.
    /// Only via `MutationGuard`.
    async fn uninstall(&self, release: &HelmReleaseRef, keep_history: bool) -> OxiResult<()>;
}
