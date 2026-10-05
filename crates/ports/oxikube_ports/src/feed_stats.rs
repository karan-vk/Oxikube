//! Watch-budget counters: [`FeedStats`], a plain snapshot of one cluster's feeds.
//!
//! The adapter's watch budget (`oxikube_kube::budget`, E04-S13) opens, shares and tears down
//! feeds per cluster and counts what flows through them. It hands those counters upward as
//! this snapshot so `oxikube_app` (E07 resource store, a status bar) and `--perf` (E01-S14)
//! can read them without depending on kube. There is no metrics exporter: a caller samples
//! [`FeedStats`] when it wants numbers and derives rates from two samples
//! ([`FeedStats::events_per_sec`]).
//!
//! Counters never carry object contents: kinds, namespaces and numbers only.

use std::time::Duration;

use oxikube_domain::ids::{ClusterId, Gvk};

/// What a feed carries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FeedVariant {
    /// Whole objects (reflector feed).
    #[default]
    Full,
    /// `PartialObjectMetadata` only: the cheap list view. Objects are
    /// [partial](oxikube_domain::Resource::is_partial).
    Metadata,
    /// Server-side Table API rows ([`TableFeed`](crate::table::TableFeed)).
    Table,
}

impl FeedVariant {
    /// Stable lowercase name, for example `metadata`. Used in logs and `--perf` output.
    pub const fn as_str(self) -> &'static str {
        match self {
            FeedVariant::Full => "full",
            FeedVariant::Metadata => "metadata",
            FeedVariant::Table => "table",
        }
    }
}

/// Counters of one open feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedStat {
    /// The watched kind.
    pub gvk: Gvk,
    /// The watched namespace; `None` for a cluster-wide feed.
    pub namespace: Option<String>,
    /// What the feed carries (after any degrade by the budget).
    pub variant: FeedVariant,
    /// Leases currently held on the feed. `0` means idle: it is torn down when its grace
    /// period ends unless someone subscribes again.
    pub subscribers: usize,
    /// Objects (or rows) the feed currently holds, from the deltas it delivered.
    pub objects: u64,
    /// `Applied` and `Deleted` deltas delivered since the feed opened.
    pub events: u64,
    /// Full lists delivered (`Restarted` deltas), the opening list included.
    pub restarts: u64,
    /// Response body bytes received for the feed (lists and watches), after decompression.
    pub bytes: u64,
    /// Error items the feed delivered (retryable or final).
    pub errors: u64,
}

impl FeedStat {
    /// Whether no lease holds the feed (it is in its grace period).
    pub fn is_idle(&self) -> bool {
        self.subscribers == 0
    }
}

/// A snapshot of one cluster's watch budget: limits, current use and cumulative counters.
///
/// Cumulative counters (`events`, `restarts`, `bytes`, `errors`) include feeds that have
/// since stopped, so two snapshots give a rate over the interval between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedStats {
    /// The cluster the feeds belong to.
    pub cluster: ClusterId,
    /// The feed cap in force.
    pub max_feeds: usize,
    /// The object cap in force.
    pub max_objects: u64,
    /// Above this many objects, new full feeds open metadata-only instead.
    pub metadata_above: u64,
    /// Feeds open now, idle ones included.
    pub feeds: usize,
    /// Feeds open now with no subscriber (in their grace period).
    pub idle_feeds: usize,
    /// Leases held now, over all feeds.
    pub subscribers: usize,
    /// Objects held now, over all open feeds.
    pub objects: u64,
    /// `Applied` and `Deleted` deltas delivered, cumulative.
    pub events: u64,
    /// Full lists delivered, cumulative.
    pub restarts: u64,
    /// Response body bytes received, cumulative.
    pub bytes: u64,
    /// Error items delivered, cumulative.
    pub errors: u64,
    /// Feeds opened, cumulative.
    pub started: u64,
    /// Feeds torn down (idle, evicted, consumer gone or ended), cumulative.
    pub stopped: u64,
    /// Full-feed requests opened metadata-only because of the object budget, cumulative.
    pub degraded: u64,
    /// Requests refused with [`BudgetExceeded`](oxikube_domain::ErrorKind::BudgetExceeded),
    /// cumulative.
    pub refused: u64,
    /// Idle feeds torn down early to make room for a new one, cumulative.
    pub evicted: u64,
    /// One entry per open feed, ordered by kind, namespace and variant.
    pub per_feed: Vec<FeedStat>,
}

impl FeedStats {
    /// An empty snapshot for `cluster` with the given limits.
    pub fn empty(cluster: ClusterId, max_feeds: usize, max_objects: u64) -> Self {
        Self {
            cluster,
            max_feeds,
            max_objects,
            metadata_above: max_objects,
            feeds: 0,
            idle_feeds: 0,
            subscribers: 0,
            objects: 0,
            events: 0,
            restarts: 0,
            bytes: 0,
            errors: 0,
            started: 0,
            stopped: 0,
            degraded: 0,
            refused: 0,
            evicted: 0,
            per_feed: Vec::new(),
        }
    }

    /// Delta events per second between `earlier` and this snapshot, `elapsed` apart.
    /// `0.0` when `elapsed` is zero.
    pub fn events_per_sec(&self, earlier: &FeedStats, elapsed: Duration) -> f64 {
        rate(self.events, earlier.events, elapsed)
    }

    /// Received bytes per second between `earlier` and this snapshot, `elapsed` apart.
    /// `0.0` when `elapsed` is zero.
    pub fn bytes_per_sec(&self, earlier: &FeedStats, elapsed: Duration) -> f64 {
        rate(self.bytes, earlier.bytes, elapsed)
    }
}

#[allow(clippy::cast_precision_loss, reason = "a rate for display")]
fn rate(now: u64, before: u64, elapsed: Duration) -> f64 {
    let secs = elapsed.as_secs_f64();
    if secs == 0.0 {
        return 0.0;
    }
    now.saturating_sub(before) as f64 / secs
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_domain::ids::ContextName;

    fn cluster() -> ClusterId {
        ClusterId::new("test", &ContextName::from("kind"))
    }

    #[test]
    fn variants_have_stable_names() {
        assert_eq!(FeedVariant::default(), FeedVariant::Full);
        assert_eq!(FeedVariant::Full.as_str(), "full");
        assert_eq!(FeedVariant::Metadata.as_str(), "metadata");
        assert_eq!(FeedVariant::Table.as_str(), "table");
    }

    #[test]
    fn rates_come_from_two_snapshots() {
        let earlier = FeedStats::empty(cluster(), 8, 100);
        let mut later = earlier.clone();
        later.events = 500;
        later.bytes = 2_048;
        let elapsed = Duration::from_secs(2);
        assert!((later.events_per_sec(&earlier, elapsed) - 250.0).abs() < f64::EPSILON);
        assert!((later.bytes_per_sec(&earlier, elapsed) - 1_024.0).abs() < f64::EPSILON);
        assert!(later.events_per_sec(&earlier, Duration::ZERO).abs() < f64::EPSILON);
        // A counter never goes backwards; a swapped pair reads as zero, not negative.
        assert!(earlier.events_per_sec(&later, elapsed).abs() < f64::EPSILON);
    }

    #[test]
    fn an_idle_feed_has_no_subscribers() {
        let stat = FeedStat {
            gvk: Gvk::new("", "v1", "Pod"),
            namespace: Some("default".into()),
            variant: FeedVariant::Metadata,
            subscribers: 0,
            objects: 3,
            events: 0,
            restarts: 1,
            bytes: 0,
            errors: 0,
        };
        assert!(stat.is_idle());
        let empty = FeedStats::empty(cluster(), 4, 10);
        assert_eq!(empty.metadata_above, 10);
        assert!(empty.per_feed.is_empty());
    }
}
