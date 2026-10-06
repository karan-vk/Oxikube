//! Store construction inputs: [`StoreRuntime`] (spawner and clock), [`StoreOptions`] (policy,
//! budget, [`StoreConfig`] tuning), and the [`FeedInfo`] diagnostic row.

use std::sync::Arc;
use std::time::Duration;

use oxikube_ports::ClockPort;

use super::budget::{FeedBudget, UnlimitedBudget};
use super::delta::FeedState;
use super::object::FeedKey;
use super::policy::{FeedKind, FeedPolicy};
use super::spawn::Spawner;

/// How long an unobserved feed keeps running (matches E04's watch budget default), so switching
/// tabs back and forth costs no relist.
pub const DEFAULT_IDLE_GRACE: Duration = Duration::from_secs(30);

/// Store tuning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreConfig {
    /// How long a feed outlives its last subscriber. Zero stops it at once.
    pub idle_grace: Duration,
    /// First delay before reopening a feed that failed with a retryable error or ended.
    pub retry_initial: Duration,
    /// Cap of the doubling retry delay.
    pub retry_max: Duration,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            idle_grace: DEFAULT_IDLE_GRACE,
            retry_initial: Duration::from_secs(1),
            retry_max: Duration::from_secs(30),
        }
    }
}

/// Instrumentation hook: what the feeds apply. The binary counts it as the feed throughput of
/// `oxikube --perf` (docs/PERFORMANCE.md); tests count batches with it.
///
/// Called on the feed's task once per batch, before the batch is applied, so keep it to an atomic
/// add: no lock, no I/O.
pub trait StoreProbe: Send + Sync {
    /// A feed is applying a batch of `events` watch events (a relist counts each object it
    /// lists).
    fn feed_batch(&self, events: usize);
}

/// Where the store runs its tasks and measures time.
#[derive(Clone)]
pub struct StoreRuntime {
    /// Runs feed drivers and grace timers (the Tokio bridge in the binary).
    pub spawner: Arc<dyn Spawner>,
    /// Grace timers and retry backoff.
    pub clock: Arc<dyn ClockPort>,
    /// Told about every batch a feed applies (`None`: nobody is counting).
    pub probe: Option<Arc<dyn StoreProbe>>,
}

impl std::fmt::Debug for StoreRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreRuntime").finish_non_exhaustive()
    }
}

/// Policy, budget and tuning; [`Default`] is the built-in policy, no budget limit and the
/// default tuning.
#[derive(Clone)]
pub struct StoreOptions {
    /// Which feed serves which kind.
    pub policy: FeedPolicy,
    /// The watch-budget hook.
    pub budget: Arc<dyn FeedBudget>,
    /// Tuning.
    pub config: StoreConfig,
}

impl Default for StoreOptions {
    fn default() -> Self {
        Self {
            policy: FeedPolicy::default(),
            budget: Arc::new(UnlimitedBudget),
            config: StoreConfig::default(),
        }
    }
}

impl std::fmt::Debug for StoreOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreOptions")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// A diagnostic view of one cached feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedInfo {
    /// Kind and scope.
    pub key: FeedKey,
    /// Which feed it is.
    pub kind: FeedKind,
    /// Live subscribers (the reference count).
    pub subscribers: usize,
    /// Cached objects.
    pub objects: usize,
    /// Its state.
    pub state: FeedState,
    /// Whether it is in its grace period (no subscriber, still running).
    pub idle: bool,
}
