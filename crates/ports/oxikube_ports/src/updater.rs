//! [`UpdaterPort`]: checking for, downloading and installing application updates.
//!
//! # Adapter
//!
//! Implemented by `oxikube_updater`, which talks to the release feed over HTTP and
//! verifies artefact signatures. HTTP stays in the adapter.
//!
//! # Mutating methods
//!
//! [`install`](UpdaterPort::install) is **mutating** (it replaces the running
//! application). It is not a cluster mutation, so it does not go through
//! `MutationGuard`; it runs only from the update flow's own explicit user
//! confirmation. [`check`](UpdaterPort::check) and [`download`](UpdaterPort::download)
//! change nothing installed.

use std::path::PathBuf;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_domain::OxiResult;

/// Which release stream to follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UpdateChannel {
    /// Tagged releases.
    #[default]
    Stable,
    /// Release candidates.
    Beta,
    /// Builds from `main`.
    Nightly,
}

/// An available update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    /// The new version.
    pub version: String,
    /// The channel it was found on.
    pub channel: UpdateChannel,
    /// Release notes, if the feed carries them.
    pub release_notes: Option<String>,
    /// When it was published.
    pub published: Option<Timestamp>,
    /// Download size.
    pub size_bytes: Option<u64>,
}

/// An update artefact downloaded and verified, ready to install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedUpdate {
    /// The version it installs.
    pub version: String,
    /// Where the verified artefact is on disk.
    pub path: PathBuf,
}

/// Application self-update.
///
/// # Effects
///
/// [`check`](Self::check) and [`download`](Self::download) change nothing installed;
/// [`install`](Self::install) is **mutating** (see the module docs).
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// [`Network`](oxikube_domain::ErrorKind::Network) /
/// [`Timeout`](oxikube_domain::ErrorKind::Timeout) (retryable) for an unreachable release feed,
/// [`Validation`](oxikube_domain::ErrorKind::Validation) for an artefact whose signature or
/// checksum does not verify, [`Internal`](oxikube_domain::ErrorKind::Internal) for local I/O
/// failures.
#[async_trait]
pub trait UpdaterPort: Send + Sync {
    /// The newest update on `channel` newer than `current_version`, or `None` when
    /// up to date.
    async fn check(
        &self,
        current_version: &str,
        channel: UpdateChannel,
    ) -> OxiResult<Option<UpdateInfo>>;

    /// Downloads and verifies `update`.
    async fn download(&self, update: &UpdateInfo) -> OxiResult<DownloadedUpdate>;

    /// **Mutating.** Installs a downloaded update, to take effect on restart. Only
    /// from the update flow after explicit user confirmation.
    async fn install(&self, update: &DownloadedUpdate) -> OxiResult<()>;
}
