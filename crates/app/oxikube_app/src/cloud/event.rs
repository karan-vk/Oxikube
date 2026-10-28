//! What a discovery run reports: [`CloudDiscoveryEvent`], and how a refresh is asked for.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::Duration;

use jiff::Timestamp;
use oxikube_domain::OxiError;
use oxikube_domain::ids::ContextName;
use oxikube_ports::{CloudDetection, CloudProvider, CloudScope, DiscoveredCluster};

/// Whether a refresh may answer from the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RefreshMode {
    /// Answer from the cache when every scope asked for was listed within the TTL; otherwise
    /// show the cached clusters at once and run the provider in the background
    /// (stale-while-revalidate).
    #[default]
    IfStale,
    /// Run the provider even when the cache is fresh (the user's "Refresh").
    Force,
}

/// One step of a [`refresh`](super::CloudDiscoveryService::refresh), in order: maybe
/// [`Cached`](Self::Cached), then [`Detected`](Self::Detected), [`Started`](Self::Started),
/// one [`ScopeDone`](Self::ScopeDone) or [`ScopeFailed`](Self::ScopeFailed) per scope in the
/// order they finish, and exactly one terminal event ([`Finished`](Self::Finished),
/// [`Failed`](Self::Failed) or [`Cancelled`](Self::Cancelled)) after which the stream ends.
///
/// Errors carry the adapter's redacted message only; nothing here holds a credential.
#[derive(Debug)]
pub enum CloudDiscoveryEvent {
    /// What the cache holds for the provider, sent first when it holds anything.
    Cached {
        /// The provider.
        provider: CloudProvider,
        /// The last detection, when one was cached.
        detection: Option<CloudDetection>,
        /// The cached clusters of the scopes asked for.
        clusters: Vec<DiscoveredCluster>,
        /// When the oldest of those listings ran.
        fetched_at: Option<Timestamp>,
        /// The cache is older than the TTL (or misses a scope), so a run follows.
        stale: bool,
    },
    /// The provider's CLI was looked for. [`NotInstalled`](oxikube_ports::CloudToolStatus) hides
    /// the provider and ends the run with an empty [`Finished`](Self::Finished); a missing auth
    /// plugin or login shows in the detection's hints and the run goes on.
    Detected(CloudDetection),
    /// The scopes about to be listed.
    Started {
        /// The provider.
        provider: CloudProvider,
        /// Every scope of this run.
        scopes: Vec<CloudScope>,
    },
    /// One scope was listed; its clusters are cached.
    ScopeDone {
        /// The scope.
        scope: CloudScope,
        /// Its clusters.
        clusters: Vec<DiscoveredCluster>,
    },
    /// One scope could not be listed. The other scopes go on, and this scope's previously
    /// cached clusters stay in the cache and in [`Finished`](Self::Finished).
    ScopeFailed {
        /// The scope.
        scope: CloudScope,
        /// Why (redacted by the adapter: not logged in, forbidden, timed out...).
        error: OxiError,
    },
    /// The run ended.
    Finished {
        /// The provider.
        provider: CloudProvider,
        /// Every cluster of the run's scopes: fresh ones, and the cached ones of failed scopes.
        clusters: Vec<DiscoveredCluster>,
        /// How many scopes failed.
        failed_scopes: usize,
    },
    /// The run could not start: detection or the scope listing failed (not logged in, ...).
    Failed {
        /// The provider.
        provider: CloudProvider,
        /// Why.
        error: OxiError,
    },
    /// [`cancel`](super::CloudDiscoveryService::cancel) or a newer refresh of the same provider
    /// stopped the run; its CLI processes were killed.
    Cancelled {
        /// The provider.
        provider: CloudProvider,
    },
}

impl CloudDiscoveryEvent {
    /// Whether this event ends its stream.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Finished { .. } | Self::Failed { .. } | Self::Cancelled { .. }
        )
    }
}

/// How discovery runs. The defaults until the `cloud.*` settings (E18-S07) set them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryConfig {
    /// How long a listing stays fresh.
    pub ttl: Duration,
    /// How many scopes of one provider are listed at once.
    pub max_parallel_scopes: NonZeroUsize,
    /// Providers switched off (`cloud.providers.*`): they are not listed by
    /// [`providers`](super::CloudDiscoveryService::providers), a refresh of one ends at once
    /// with no clusters, and its running refresh is cancelled.
    pub disabled: BTreeSet<CloudProvider>,
}

impl DiscoveryConfig {
    /// Fifteen minutes: clusters come and go rarely, and `gcloud` is slow.
    pub const DEFAULT_TTL: Duration = Duration::from_secs(15 * 60);
    /// Four scopes at a time (the runner caps the processes of all providers on top).
    pub const DEFAULT_MAX_PARALLEL_SCOPES: NonZeroUsize = NonZeroUsize::new(4).expect("4 > 0");
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            ttl: Self::DEFAULT_TTL,
            max_parallel_scopes: Self::DEFAULT_MAX_PARALLEL_SCOPES,
            disabled: BTreeSet::new(),
        }
    }
}

/// What [`add`](super::CloudDiscoveryService::add) did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedCluster {
    /// The context the cluster is reached through.
    pub context: ContextName,
    /// The managed kubeconfig that holds it.
    pub path: PathBuf,
    /// The file already held exactly this entry, so it was not rewritten.
    pub unchanged: bool,
}

/// What [`remove`](super::CloudDiscoveryService::remove) did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedCluster {
    /// The context that was removed.
    pub context: ContextName,
    /// The managed kubeconfig it was removed from.
    pub path: PathBuf,
    /// It was the file's last cluster, so the file and its source entry were deleted.
    pub deleted_file: bool,
    /// An open session of that context was disconnected.
    pub disconnected: bool,
}
