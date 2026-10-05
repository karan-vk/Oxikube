//! [`BudgetConfig`]: the per-cluster limits and the idle grace period.

use std::time::Duration;

/// Default [`BudgetConfig::max_feeds`].
pub const DEFAULT_MAX_FEEDS: usize = 64;

/// Default [`BudgetConfig::max_objects`].
pub const DEFAULT_MAX_OBJECTS: u64 = 100_000;

/// Default [`BudgetConfig::metadata_above`].
pub const DEFAULT_METADATA_ABOVE: u64 = 25_000;

/// Default [`BudgetConfig::idle_grace`].
pub const DEFAULT_IDLE_GRACE: Duration = Duration::from_secs(30);

/// Limits of one cluster's watch budget. The values come from the per-cluster settings layer
/// (E06-S08); [`FeedRegistry::set_config`](super::FeedRegistry::set_config) applies a change.
///
/// Limits gate *new* feeds. A feed already open is never truncated, whatever it grows to:
/// objects are never dropped silently. The order on a breach is: tear down idle feeds
/// (oldest first), then open a requested full feed metadata-only, then refuse with
/// [`ErrorKind::BudgetExceeded`](oxikube_domain::ErrorKind::BudgetExceeded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetConfig {
    /// Most feeds open at once, idle ones included. Each namespace of a namespace set is
    /// its own feed. Default [`DEFAULT_MAX_FEEDS`].
    pub max_feeds: usize,
    /// No new feed opens while the open feeds hold this many objects. Default
    /// [`DEFAULT_MAX_OBJECTS`].
    pub max_objects: u64,
    /// While the open feeds hold this many objects, a request for a full feed opens a
    /// metadata-only feed instead (E04-S03). Default [`DEFAULT_METADATA_ABOVE`]; set it to
    /// `max_objects` or more to never degrade.
    pub metadata_above: u64,
    /// How long a feed with no subscriber stays open before it is torn down. Subscribing again
    /// within it reuses the feed, so a tab switch stays instant. `Duration::ZERO` tears down
    /// at once. Default [`DEFAULT_IDLE_GRACE`].
    pub idle_grace: Duration,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_feeds: DEFAULT_MAX_FEEDS,
            max_objects: DEFAULT_MAX_OBJECTS,
            metadata_above: DEFAULT_METADATA_ABOVE,
            idle_grace: DEFAULT_IDLE_GRACE,
        }
    }
}
