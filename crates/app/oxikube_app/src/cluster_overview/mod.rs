//! The cluster overview's numbers (E13-S09): health, capacity and allocation, top consumers and
//! the distribution guess, kept incrementally off the UI thread; and its history graphs.
//!
//! | File | Holds |
//! |---|---|
//! | `feed` | [`OverviewFeed`]: the driver that follows the cluster-wide Pod and Node feeds and the live metrics, and publishes an [`OverviewSnapshot`] after each batch |
//! | `mirror` | a subscription's rows as the feed keeps them, and the tallies' row hooks |
//! | `pods` | [`PodTally`], [`PhaseCounts`]: pods by phase and the demand of the pods on nodes |
//! | `nodes` | [`NodeTally`], [`NodeCounts`], [`Capacity`]: nodes by readiness and their allocatable sum |
//! | `top` | [`top_n`], [`TopHeap`], [`top_consumers`]: the largest pods and namespaces of a metrics poll, with a bounded heap |
//! | `distribution` | [`Distribution`], [`detect_distribution`]: EKS, GKE, AKS, OpenShift, k3s, kind... from the version string and the nodes, "unknown" otherwise |
//! | `snapshot` | [`OverviewSnapshot`], [`FeedStatus`], [`UsageTotals`] |
//! | `history` | [`fetch_cluster_history`], [`HistoryAvailability`], [`ProviderSource`]: the Prometheus graphs' data and where their availability comes from |
//!
//! # Cost
//!
//! Nothing here runs per frame. A pod's change costs one [`PodPlacement`](oxikube_domain::PodPlacement)
//! (a walk over its containers) and a few hash operations; a node's change one
//! [`NodeSummary`](oxikube_domain::NodeSummary) and the distribution rules over the nodes (a
//! hundred at most in practice); a metrics poll one pass over the pods' latest readings with
//! bounded heaps. A snapshot of the whole Pod feed (the first list, a relist) is counted once from
//! scratch. The publish is a handful of integers and the top lists, behind an `Arc`.
//!
//! # Absence
//!
//! A feed that is forbidden or failed says so ([`FeedStatus::problem`]); metrics that are not
//! there carry their [`MetricsState`](crate::MetricsState), and the usage and top lists are
//! empty rather than zero.

mod distribution;
mod feed;
mod history;
mod mirror;
mod nodes;
mod pods;
mod snapshot;
mod top;

#[cfg(test)]
mod tests;

pub use distribution::{Distribution, Evidence, RULES, detect_distribution};
pub use feed::{OverviewFeed, OverviewInput, StartedFeed, TOP_N};
pub use history::{
    ClusterHistory, HISTORY_SPAN, HistoryAvailability, HistoryLine, ProviderSource,
    fetch_cluster_history,
};
pub use nodes::{Capacity, NodeCounts, NodeTally};
pub use pods::{PhaseCounts, PodTally};
pub use snapshot::{FeedStatus, OverviewSnapshot, UsageTotals};
pub use top::{TopConsumers, TopEntry, TopHeap, top_consumers, top_n};
