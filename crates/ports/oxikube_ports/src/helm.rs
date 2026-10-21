//! [`HelmPort`]: Helm releases of a cluster, read natively with the `helm` CLI for the rest.
//!
//! The vocabulary ([`Release`], [`Revision`], [`ChartRef`], [`RepoEntry`], [`Values`], ...)
//! lives in [`oxikube_domain::helm`] and is re-exported here.
//!
//! # Adapters
//!
//! One trait, two implementations that each cover a subset (research 2.5):
//!
//! * the **native reader** (`oxikube_helm`) decodes the release Secrets/ConfigMaps
//!   (`sh.helm.release.v1.*`): `list_releases`, `history`, `values`, `manifest`, `notes`;
//! * the **CLI adapter** (`oxikube_helm`) drives the user's `helm` binary: everything,
//!   and the only source of `repos`, `search`, `show`, `template`, `install`, `upgrade`,
//!   `rollback` and `uninstall`.
//!
//! An adapter that lacks an operation returns [`HelmError::Unsupported`] (as
//! [`OxiError`](oxikube_domain::OxiError), kind `Unsupported`). That is how native-only
//! mode works when no binary is installed. [`capabilities`](HelmPort::capabilities) tells
//! the UI and tools up front which operations will work, so they are hidden rather than
//! tried.
//!
//! # Mutating methods
//!
//! [`install`](HelmPort::install), [`upgrade`](HelmPort::upgrade),
//! [`rollback`](HelmPort::rollback) and [`uninstall`](HelmPort::uninstall) are
//! **mutating**. They are reachable only through `MutationGuard` (read-only mode,
//! confirmation tier, dry-run where Helm supports it, audit); the UI never calls them
//! directly. [`template`](HelmPort::template) renders locally and does not mutate. All
//! other methods are read-only.
//!
//! # Sensitivity
//!
//! Values, manifests and command output can embed credentials. Callers show them but
//! never persist them (non-negotiable 5); adapters redact output lines and error text.

use async_trait::async_trait;
use futures::stream::BoxStream;
use oxikube_domain::OxiResult;
pub use oxikube_domain::helm::{
    ChartHit, ChartRef, HelmCapabilities, HelmError, HelmVersion, InstallRequest, OutputKind,
    OutputLine, Release, ReleaseRef, ReleaseStatus, RepoAddRequest, RepoEntry, RepoPassword,
    Revision, RollbackRequest, SearchQuery, ShowKind, UninstallRequest, UpgradeRequest, Values,
};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;

/// The progress of an install or upgrade: [`OutputLine`]s in order, then the end of the
/// stream when Helm succeeded, or a single `Err` that ends it when Helm failed.
///
/// Nothing happens until the stream is polled; dropping it kills the `helm` process.
pub type HelmOutput = BoxStream<'static, OxiResult<OutputLine>>;

/// Helm access for a cluster.
///
/// # Effects
///
/// Read-only except [`install`](Self::install), [`upgrade`](Self::upgrade),
/// [`rollback`](Self::rollback) and [`uninstall`](Self::uninstall), which are
/// **mutating** (see the module docs).
///
/// # Errors
///
/// Adapters map failures with [`HelmError`] and the table in `docs/ARCHITECTURE.md`.
/// Expected kinds: [`NotFound`](oxikube_domain::ErrorKind::NotFound) for an unknown
/// release or revision, [`Forbidden`](oxikube_domain::ErrorKind::Forbidden) when RBAC
/// denies reading release Secrets, [`Unsupported`](oxikube_domain::ErrorKind::Unsupported)
/// when the operation needs a `helm` binary that is missing or too old (or the adapter
/// lacks it), [`Network`](oxikube_domain::ErrorKind::Network) /
/// [`Timeout`](oxikube_domain::ErrorKind::Timeout) (retryable) for connection failures.
#[async_trait]
pub trait HelmPort: Send + Sync {
    // --- read-only, native-capable ---------------------------------------------------

    /// The latest revision of every release in the selected namespaces. List-shaped:
    /// `notes` is `None` and both value trees are empty; use [`notes`](Self::notes) and
    /// [`values`](Self::values) for those.
    async fn list_releases(
        &self,
        cluster: &ClusterId,
        namespaces: &NamespaceSelection,
    ) -> OxiResult<Vec<Release>>;

    /// Every revision of `release`, newest first.
    async fn history(&self, release: &ReleaseRef) -> OxiResult<Vec<Revision>>;

    /// The values of a revision (latest when `None`): the user-supplied ones, or with `all`
    /// the chart defaults merged in. May contain secrets; do not persist.
    async fn values(
        &self,
        release: &ReleaseRef,
        revision: Option<u32>,
        all: bool,
    ) -> OxiResult<Values>;

    /// The rendered manifest of a revision (latest when `None`). May contain secrets; do
    /// not persist.
    async fn manifest(&self, release: &ReleaseRef, revision: Option<u32>) -> OxiResult<String>;

    /// The rendered `NOTES.txt` of a revision (latest when `None`), `None` if the chart
    /// has none.
    async fn notes(&self, release: &ReleaseRef, revision: Option<u32>)
    -> OxiResult<Option<String>>;

    // --- CLI-only, read-only ---------------------------------------------------------

    /// The configured chart repositories (`helm repo list`).
    async fn repos(&self) -> OxiResult<Vec<RepoEntry>>;

    /// Searches the configured repositories for charts (`helm search repo`).
    async fn search(&self, query: &SearchQuery) -> OxiResult<Vec<ChartHit>>;

    /// Prints part of a chart (`helm show`): its metadata, README, default values or CRDs.
    /// `chart.repo` names the repository.
    async fn show(&self, chart: &ChartRef, kind: ShowKind) -> OxiResult<String>;

    /// Renders the manifest `request` would install (`helm template`), changing nothing.
    async fn template(&self, request: &InstallRequest) -> OxiResult<String>;

    // --- CLI-only, mutating ----------------------------------------------------------

    /// **Mutating.** Installs a chart as a new release. Only via `MutationGuard`.
    /// The stream is the install's progress; see [`HelmOutput`].
    async fn install(&self, request: &InstallRequest) -> OxiResult<HelmOutput>;

    /// **Mutating.** Upgrades a release. Only via `MutationGuard`. The stream is the
    /// upgrade's progress; see [`HelmOutput`].
    async fn upgrade(&self, request: &UpgradeRequest) -> OxiResult<HelmOutput>;

    /// **Mutating.** Rolls a release back to an earlier revision. Only via `MutationGuard`.
    async fn rollback(&self, request: &RollbackRequest) -> OxiResult<()>;

    /// **Mutating.** Uninstalls a release, keeping its history when asked. Only via
    /// `MutationGuard`.
    async fn uninstall(&self, request: &UninstallRequest) -> OxiResult<()>;

    // --- capabilities ----------------------------------------------------------------

    /// What this adapter can do right now. Synchronous and cheap: adapters detect the
    /// binary ahead of time and cache the answer. Views and tools gate on it.
    fn capabilities(&self) -> HelmCapabilities;
}

/// Management of the chart repositories in Helm's own configuration (`repositories.yaml`),
/// implemented by `oxikube_helm` over the `helm` binary.
///
/// Listing is [`HelmPort::repos`]. These methods change only the local Helm configuration, not
/// a cluster, so they are not `MutationGuard` mutations. They need the `helm` binary: without
/// one they fail with [`HelmError::BinaryMissing`] (kind `Unsupported`). Repositories are never
/// copied anywhere else, so the CLI, other tools and Oxikube always agree. A password is handed
/// to `helm` on its stdin, never its command line, and never logged.
///
/// # Errors
///
/// [`CliFailed`](HelmError::CliFailed) when `helm` rejects the operation (unreachable URL, 401,
/// bad index), with a redacted summary of what it said.
#[async_trait]
pub trait HelmRepoPort: Send + Sync {
    /// **Changes local Helm configuration.** Adds (and fetches the index of) a repository
    /// (`helm repo add`).
    async fn repo_add(&self, request: &RepoAddRequest) -> OxiResult<()>;

    /// **Changes local Helm configuration.** Removes the repository `name`
    /// (`helm repo remove`).
    async fn repo_remove(&self, name: &str) -> OxiResult<()>;

    /// Refreshes the index of each repository in `names` (`helm repo update`); every
    /// repository when `names` is empty.
    async fn repo_update(&self, names: &[String]) -> OxiResult<()>;
}
