//! The node table's usage cells: CPU and Mem (what the node uses, k9s' `CPU` and `MEM`) and
//! CPU% and Mem% (that usage over the node's allocatable, k9s' `%CPU` and `%MEM`), all read from
//! a [`NodeUsageSource`] E13 registers.

use oxikube_domain::{Quantity, Resource};

use crate::columns::{Cell, CellSort, UsageThresholds};
use crate::metrics::live::memory_text;

/// A node's current CPU and memory usage, from the metrics API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeUsage {
    /// CPU in use, in millicores.
    pub cpu_millis: i64,
    /// Memory in use, in bytes.
    pub memory_bytes: i64,
}

/// Where the CPU, Mem, CPU% and Mem% cells get a node's usage (E13 implements it over its
/// metrics store).
pub trait NodeUsageSource: Send + Sync {
    /// The latest usage of the node named `node`; `None` when there is no sample (yet).
    fn usage(&self, node: &str) -> Option<NodeUsage>;

    /// A number that changes whenever some node's usage may have (a new sample). A table sorted
    /// by a usage column ranks again when it moves; a source that never reports `0` leaves a
    /// sort by usage as stale as its last ranking.
    fn epoch(&self) -> u64 {
        0
    }

    /// When CPU% and Mem% turn amber or red (the `metrics.thresholds` setting). The default is
    /// k9s': warn 70, critical 90.
    fn thresholds(&self) -> UsageThresholds {
        UsageThresholds::default()
    }
}

/// Shown in a cell that has no value (no Pod feed yet, no metrics).
pub(super) const ABSENT: &str = "–";

/// The cell shown when a value is absent: `–`, sorted last.
pub(super) fn absent<'a>() -> Cell<'a> {
    Cell::shown(ABSENT, CellSort::None)
}

/// Which usage a cell shows.
#[derive(Debug, Clone, Copy)]
pub(super) struct UsageColumn {
    /// CPU (`true`) or memory.
    pub cpu: bool,
    /// Over allocatable (`true`) or the absolute amount.
    pub percent: bool,
}

/// The usage cell of `res`: `1850m` / `3204Mi` sorted by the exact quantity, or `46%` toned by
/// the source's thresholds; `–` without a source, a sample or (for `%`) an allocatable amount.
pub(super) fn usage_cell<'a>(
    source: Option<&dyn NodeUsageSource>,
    res: &Resource,
    column: UsageColumn,
) -> Cell<'a> {
    let Some(source) = source else {
        return absent();
    };
    let Some(usage) = source.usage(&res.meta.name) else {
        return absent();
    };
    let UsageColumn { cpu, percent } = column;
    let used = if cpu {
        Quantity::from_milli(usage.cpu_millis)
    } else {
        Quantity::from_value(usage.memory_bytes)
    };
    if !percent {
        let text = if cpu {
            format!("{}m", usage.cpu_millis)
        } else {
            memory_text(u64::try_from(usage.memory_bytes).unwrap_or(0))
        };
        return Cell::quantity(text, used);
    }
    let allocatable = res
        .json()
        .get("status")
        .and_then(|s| s.get("allocatable"))
        .and_then(|a| a.get(if cpu { "cpu" } else { "memory" }))
        .and_then(|q| q.as_str())
        .and_then(|q| Quantity::parse(q).ok());
    let Some(allocatable) = allocatable else {
        return absent();
    };
    match used.percent_of(&allocatable) {
        Some(percent) => Cell::float(format!("{}%", percent.round()), percent)
            .with_tone(source.thresholds().of(cpu).tone(percent)),
        None => absent(),
    }
}
