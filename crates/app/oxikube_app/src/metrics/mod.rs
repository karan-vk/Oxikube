//! Utilisation for the metrics columns, overview and tools (E13-S02): what is allocated on each
//! node and in each namespace, over snapshots of the resource store.
//!
//! The pure per-pod maths (requests and limits, `%CPU/R`-style percents, the edge-case rules) is
//! in [`oxikube_domain::utilisation`]; this module sums it over many objects. Everything here is
//! a function over `&Resource` snapshots, with no I/O, so the polling service (E13-S03), the
//! allocation views (E13-S11) and the agent tools call the same code and show the same numbers.
//!
//! | Item | Holds |
//! |---|---|
//! | [`UtilisationAggregator`] | the entry point: nodes, namespaces and pods, using the GPU vendor list of the cluster at the time of the call |
//! | [`GpuVendorsHandle`] | the hot-reloadable `metrics.gpu_vendors` table the aggregator reads (the binary pushes settings changes into it) |
//! | [`NodeAllocation`] | one node: allocatable, the requests and limits of the pods on it, pod count |
//! | [`cluster_utilisation`] (E13-S10) | the cluster's CPU and memory use against the nodes' allocatable, with its history from the per-node rings: the pulses gauges |
//! | [`NamespaceAllocation`] | one namespace: pod count, requests and limits, compared with a quota's `hard` amounts |
//! | [`live`] (E13-S03) | [`MetricsService`]: live CPU and memory from metrics-server, polled per cluster while someone looks, kept per node and pod in [`ClusterUsage`] (the metrics columns' source) |
//!
//! # Rules
//!
//! The rules are those of the domain module: terminated pods hold nothing, unscheduled pods count
//! towards their namespace (as a `ResourceQuota` counts them) but towards no node, a pod without
//! requests contributes zero, and a percent over a zero base is `None`. Objects that are not pods
//! (or not nodes) in an input are skipped, never an error: a snapshot may be a mixed list.
//!
//! # Performance
//!
//! One pass over the pods with a node index (`HashMap`), allocation-light: a pod without GPUs
//! allocates nothing beyond its `Arc` clones. `cargo bench -p oxikube_app --bench utilisation`
//! measures 10 000 pods on 100 nodes in 50 namespaces (release, Apple silicon): one pod's requests and
//! limits ~0.65 us, its four percents ~50 ns, a whole-cluster node or namespace pass ~8 ms. The
//! per-row figures are cheap enough for the visible rows of a table on the UI thread; a
//! whole-cluster pass belongs on a background task (the polling service and the allocation views
//! call it there), never in a frame.

mod aggregator;
mod cluster;
pub mod live;
mod namespaces;
mod nodes;
#[cfg(test)]
mod tests;
mod vendors;

pub use aggregator::UtilisationAggregator;
pub use cluster::{ClusterUtilisation, ResourceUse, cluster_utilisation};
pub use live::{
    ClusterUsage, LiveMetricsConfig, LiveMetricsConfigTable, MetricsInterest, MetricsKind,
    MetricsService, MetricsState, MetricsVisibility,
};
pub use namespaces::{NamespaceAllocation, allocate_namespaces};
pub use nodes::{NodeAllocation, allocate_nodes};
pub use vendors::GpuVendorsHandle;
