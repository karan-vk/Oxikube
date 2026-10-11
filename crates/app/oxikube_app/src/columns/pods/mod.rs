//! [`PodColumns`]: the Pod kind's own [`ColumnProvider`] (E12-S02), registered through its
//! `KindSpec` so the pods table shows what `kubectl get pods` shows plus what Lens and k9s add.
//!
//! | Column | Cell |
//! |---|---|
//! | Name, Namespace | metadata |
//! | Ready | `ready/total` containers, sorted by the ratio |
//! | Status | kubectl v1.36's printer ([`PodSummary::status`]), toned by meaning |
//! | Restarts | `3 (5m ago)`, sorted by the count |
//! | CPU, Memory | usage from the metrics source (E13); `–` without one or without a sample |
//! | %CPU/R, %CPU/L, %MEM/R, %MEM/L | usage over the pod's effective request or limit; `–` when either is missing |
//! | IP, Node, QoS (hidden by default, as in Freelens) | the summary |
//! | Service Account, Controlled By, Age | the summary, metadata |
//! | Nominated Node, Last Restart, Labels (hidden) | the summary, metadata |
//!
//! Every computed cell reads one [`PodSummary`], derived once per object version and memoised
//! (`memo`), never per cell or per frame. The column set does not depend on the session's
//! capabilities: the usage columns always show, `–` while metrics are absent.
//!
//! # Metrics (E13)
//!
//! A [`MetricsSource`] set with [`PodColumns::set_metrics`] fills CPU and Memory with its cells
//! and the `%` columns from those cells' [`CellSort::Quantity`](crate::columns::CellSort)
//! values (CPU in cores, memory in bytes), so a source only has to give a usage cell its exact
//! quantity as the sort key.

mod cells;
mod defs;
mod memo;

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{Capabilities, PodSummary, Resource};
use parking_lot::RwLock;

pub use cells::ABSENT;
pub(crate) use cells::cell as summary_cell;
pub use defs::*;

use super::builtin::{MetricsSource, meta_cell};
use super::{Cell, Column, ColumnId, ColumnProvider};
use crate::store::StoreObject;
use memo::SummaryMemo;

/// The pods table's columns. See the [module docs](self).
pub struct PodColumns {
    columns: Arc<[Column]>,
    memo: SummaryMemo,
    metrics: RwLock<Option<Arc<dyn MetricsSource>>>,
}

impl Default for PodColumns {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for PodColumns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PodColumns")
            .field("columns", &self.columns.len())
            .field("metrics", &self.metrics.read().is_some())
            .finish_non_exhaustive()
    }
}

impl PodColumns {
    /// The columns, without a metrics source (usage reads `–`).
    pub fn new() -> Self {
        Self {
            columns: defs::columns(),
            memo: SummaryMemo::default(),
            metrics: RwLock::new(None),
        }
    }

    /// The same columns reading usage from `source`.
    #[must_use]
    pub fn with_metrics(self, source: Arc<dyn MetricsSource>) -> Self {
        self.set_metrics(Some(source));
        self
    }

    /// Sets (or, with `None`, clears) the metrics source. Takes effect on the next cell read; a
    /// table redraws its cells when the second turns (they carry ages) or a row changes.
    pub fn set_metrics(&self, source: Option<Arc<dyn MetricsSource>>) {
        *self.metrics.write() = source;
    }

    /// The memoised summary of `pod` (`None` for a metadata-only object or another kind).
    pub fn summary(&self, pod: &Resource) -> Option<Arc<PodSummary>> {
        self.memo.get(pod)
    }

    /// How many pod summaries were derived so far: once per object version read, however many
    /// cells, frames and sorts read it (the per-delta derivation count `--perf` reports).
    pub fn derivations(&self) -> u64 {
        self.memo.derived()
    }

    /// The cell of a full or metadata-only Pod.
    fn resource_cell<'a>(&self, pod: &'a Resource, column: &ColumnId, now: Timestamp) -> Cell<'a> {
        let id = column.as_str();
        if matches!(
            id,
            ColumnId::NAME | ColumnId::NAMESPACE | ColumnId::AGE | ColumnId::LABELS
        ) {
            return meta_cell(&pod.meta, column, now);
        }
        let Some(summary) = self.memo.get(pod) else {
            return Cell::empty();
        };
        let metrics = self.metrics.read();
        cells::cell(id, pod, &summary, metrics.as_deref(), now).unwrap_or_else(Cell::empty)
    }
}

impl ColumnProvider for PodColumns {
    fn columns(&self, _kind: &Gvk, _caps: Capabilities) -> Arc<[Column]> {
        self.columns.clone()
    }

    fn cell<'a>(&self, object: &'a StoreObject, column: &ColumnId, now: Timestamp) -> Cell<'a> {
        match object {
            StoreObject::Resource(pod) => self.resource_cell(pod, column, now),
            StoreObject::Row(row) => meta_cell(&row.meta, column, now),
        }
    }
}

#[cfg(test)]
mod tests;
