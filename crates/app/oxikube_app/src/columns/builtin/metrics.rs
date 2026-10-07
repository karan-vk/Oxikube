//! The metrics hook: how E13 supplies CPU and memory cells without changing the provider API.

use jiff::Timestamp;
use oxikube_domain::Resource;

use super::def::Metric;
use crate::columns::Cell;

/// Supplies the CPU and memory cells of core columns once metrics exist (E13).
///
/// Until a source is registered with [`CoreColumns::with_metrics`](super::CoreColumns::with_metrics),
/// the CPU and Memory columns are not offered. For an object the source has no sample for, the
/// cell is [`Cell::Pending`]: blank, never `0`.
pub trait MetricsSource: Send + Sync {
    /// The cell of `metric` for `object`, or `None` when there is no sample (yet).
    fn cell(&self, object: &Resource, metric: Metric, now: Timestamp) -> Option<Cell<'static>>;
}
