//! The metrics hook: how E13 supplies CPU and memory cells without changing the provider API.

use jiff::Timestamp;
use oxikube_domain::Resource;

use super::def::Metric;
use crate::columns::{Cell, UsageThresholds};

/// Supplies the CPU and memory cells of core columns once metrics exist (E13).
///
/// Until a source is registered with [`CoreColumns::with_metrics`](super::CoreColumns::with_metrics),
/// the CPU and Memory columns are not offered. For an object the source has no sample for, the
/// cell is [`Cell::Pending`]: blank, never `0`.
///
/// `oxikube_app::metrics::ClusterUsage` (E13-S03) is the app's implementation: one per cluster,
/// over the samples the `MetricsService` polls.
pub trait MetricsSource: Send + Sync {
    /// The cell of `metric` for `object`, or `None` when there is no sample (yet). A usage cell
    /// carries the exact usage as its [`CellSort::Quantity`](crate::columns::CellSort) key (CPU
    /// in cores, memory in bytes): the `%` columns divide it.
    fn cell(&self, object: &Resource, metric: Metric, now: Timestamp) -> Option<Cell<'static>>;

    /// When a `%` of a limit turns amber or red (the `metrics.thresholds` setting). The default
    /// is k9s': warn 70, critical 90.
    fn thresholds(&self) -> UsageThresholds {
        UsageThresholds::default()
    }

    /// A number that changes whenever some sample may have (a new poll). A table sorted by a
    /// usage column ranks again when it moves (see
    /// [`ColumnProvider::rank_epoch`](crate::columns::ColumnProvider::rank_epoch)). `0`, the
    /// default, is a source whose cells never move.
    fn epoch(&self) -> u64 {
        0
    }
}
