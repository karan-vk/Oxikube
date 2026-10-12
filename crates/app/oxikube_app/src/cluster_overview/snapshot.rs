//! [`OverviewSnapshot`]: everything the overview's tiles, bars and top lists show, as one value
//! the feed publishes and the view reads without a lock.

use oxikube_domain::Demand;

use super::distribution::Distribution;
use super::nodes::{Capacity, NodeCounts};
use super::pods::PhaseCounts;
use super::top::TopConsumers;
use crate::metrics::MetricsState;
use crate::store::FeedState;

/// Where a kind's numbers come from: the feed's state, and whether its objects are complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedStatus {
    /// The feed's state (`Warming` until the first list).
    pub state: FeedState,
    /// The watch budget degraded the feed to metadata or a server table, so phases and requests
    /// cannot be read: the counts are totals only.
    pub partial: bool,
}

impl Default for FeedStatus {
    fn default() -> Self {
        Self {
            state: FeedState::Warming,
            partial: false,
        }
    }
}

impl FeedStatus {
    /// Whether the numbers can be shown: listed (or kept while retrying).
    pub fn is_listed(&self) -> bool {
        matches!(self.state, FeedState::Ready | FeedState::Retrying { .. })
    }

    /// Why there are no numbers, when the feed stopped: the server's or the budget's reason.
    pub fn problem(&self) -> Option<String> {
        match &self.state {
            FeedState::Forbidden { message } => Some(format!("no access: {message}")),
            FeedState::Unauthorized { message } => Some(format!("not signed in: {message}")),
            FeedState::Failed { message, .. } => Some(message.clone()),
            FeedState::Warming | FeedState::Ready | FeedState::Retrying { .. } => None,
        }
    }
}

/// Live CPU and memory summed over the nodes metrics-server reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UsageTotals {
    /// CPU in nanocores.
    pub cpu_nanocores: u64,
    /// Working-set memory in bytes.
    pub memory_bytes: u64,
    /// Nodes with a reading.
    pub nodes: usize,
}

impl UsageTotals {
    /// CPU in millicores, the unit of the allocation bars.
    pub fn cpu_millis(&self) -> i64 {
        i64::try_from(self.cpu_nanocores / 1_000_000).unwrap_or(i64::MAX)
    }

    /// Memory in bytes as the bars' integer type.
    pub fn memory(&self) -> i64 {
        i64::try_from(self.memory_bytes).unwrap_or(i64::MAX)
    }
}

/// What the cluster overview shows, published by [`OverviewFeed`](super::OverviewFeed) after
/// every batch of changes. A value: the view clones the `Arc` and renders from it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OverviewSnapshot {
    /// The cluster-wide Pod feed.
    pub pods_feed: FeedStatus,
    /// Pods by phase.
    pub phases: PhaseCounts,
    /// What the pods on nodes request and are limited to.
    pub demand: Demand,
    /// The Node feed.
    pub nodes_feed: FeedStatus,
    /// Nodes by readiness.
    pub nodes: NodeCounts,
    /// What the nodes can allocate.
    pub capacity: Capacity,
    /// The distribution the server version and the nodes point to.
    pub distribution: Distribution,
    /// The node metrics poll.
    pub node_metrics: MetricsState,
    /// The pod metrics poll.
    pub pod_metrics: MetricsState,
    /// Usage summed over the nodes, while node metrics are available.
    pub usage: Option<UsageTotals>,
    /// The largest consumers of the last pod poll (empty without pod metrics).
    pub top: TopConsumers,
    /// Moves with every publish (a batch that changes nothing shown is not published).
    pub generation: u64,
}

impl OverviewSnapshot {
    /// Whether `other` shows the same: every field but [`generation`](Self::generation).
    pub fn same_numbers(&self, other: &Self) -> bool {
        let Self {
            pods_feed,
            phases,
            demand,
            nodes_feed,
            nodes,
            capacity,
            distribution,
            node_metrics,
            pod_metrics,
            usage,
            top,
            generation: _,
        } = self;
        *pods_feed == other.pods_feed
            && *phases == other.phases
            && *demand == other.demand
            && *nodes_feed == other.nodes_feed
            && *nodes == other.nodes
            && *capacity == other.capacity
            && *distribution == other.distribution
            && *node_metrics == other.node_metrics
            && *pod_metrics == other.pod_metrics
            && *usage == other.usage
            && *top == other.top
    }
}
