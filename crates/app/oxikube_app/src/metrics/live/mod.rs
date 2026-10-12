//! Live CPU and memory from metrics-server (E13-S03): [`MetricsService`] polls each cluster
//! someone is looking at, and [`ClusterUsage`] holds what it read for the table columns.
//!
//! | Item | Holds |
//! |---|---|
//! | [`MetricsService`] | one poller per watched cluster; [`watch`](MetricsService::watch) registers a consumer of a [`MetricsKind`], [`visibility`](MetricsService::visibility) follows the cluster tab a window shows, [`set_config`](MetricsService::set_config) the `metrics` setting |
//! | [`ClusterUsage`] | one cluster's samples: the latest and the last [`RING_CAPACITY`] per node and pod ([`SampleRing`]), each pod's containers, the [`MetricsState`] per kind; the Pods table's `MetricsSource` and the Nodes table's `NodeUsageSource` |
//! | [`MetricsState`] | `Idle`, `Loading`, `Available`, `Unavailable(reason)` or `Failed`: `metrics unavailable: <reason>` is shown, never a zero |
//! | [`Cadence`] | when a kind is polled next: the interval, backoff after failures, rarely while the API is absent |
//! | [`ContainerUsageCells`] | a container row's CPU, MEM and `%` cells |
//!
//! # When a cluster is polled
//!
//! Only while all of these hold: a consumer holds a [`MetricsInterest`] for the kind (a Pods or
//! Nodes table that is the active item of its pane, a pod's open detail), a window shows the
//! cluster's tab ([`MetricsVisibility`]), and the session is connected. The poller is a task on
//! the binary's Tokio bridge held by an abort-on-drop guard: the last consumer gone, the tab
//! hidden or the connection dropped, the guard is dropped and the call in flight with it. Only
//! the watched kinds are polled (`NodeMetrics`, `PodMetrics` in the session's namespaces): a
//! Nodes table never lists every pod of a big cluster.
//!
//! # Publishing
//!
//! Each poll lands in [`ClusterUsage`] as one batch and moves its epoch once; a table reads it
//! through its once-a-second redraw check and ranks a sort by usage again when the epoch moved,
//! so a poll costs at most one redraw per table, never one per sample. [`changes`] yields once
//! per poll for views that follow it (a pod's containers).
//!
//! [`changes`]: MetricsService::changes

mod cadence;
mod config;
mod containers;
mod handles;
mod poller;
mod reconcile;
mod ring;
mod service;
mod state;
mod usage;

#[cfg(test)]
mod tests;

pub use cadence::{Cadence, MAX_BACKOFF, UNAVAILABLE_INTERVAL};
pub use config::{
    DEFAULT_POLL_INTERVAL, LiveMetricsConfig, LiveMetricsConfigTable, MAX_POLL_INTERVAL,
    MIN_POLL_INTERVAL,
};
pub use containers::ContainerUsageCells;
pub use handles::{MetricsInterest, MetricsVisibility};
pub use ring::{RING_CAPACITY, SampleRing, UsagePoint};
pub use service::MetricsService;
pub use state::{MetricsKind, MetricsState};
pub use usage::{ClusterUsage, cpu_text, memory_text, usage_cell};
