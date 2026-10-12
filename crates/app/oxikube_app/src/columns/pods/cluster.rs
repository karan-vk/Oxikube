//! [`ClusterPodColumns`]: the shared [`PodColumns`] with one cluster's usage source.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::Capabilities;
use oxikube_domain::ids::Gvk;

use super::{PodColumns, defs};
use crate::columns::builtin::MetricsSource;
use crate::columns::{Cell, Column, ColumnId, ColumnProvider};
use crate::store::StoreObject;

/// The pod columns of one cluster's table: every cell is the shared provider's (one summary memo
/// for every cluster), with CPU, Memory and the `%` columns read from this cluster's source.
/// Made by [`PodColumns::in_cluster`].
pub struct ClusterPodColumns {
    shared: Arc<PodColumns>,
    metrics: Option<Arc<dyn MetricsSource>>,
}

impl ClusterPodColumns {
    pub(super) fn new(shared: Arc<PodColumns>, metrics: Option<Arc<dyn MetricsSource>>) -> Self {
        Self { shared, metrics }
    }
}

impl std::fmt::Debug for ClusterPodColumns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterPodColumns")
            .field("metrics", &self.metrics.is_some())
            .finish_non_exhaustive()
    }
}

impl ColumnProvider for ClusterPodColumns {
    fn columns(&self, kind: &Gvk, caps: Capabilities) -> Arc<[Column]> {
        self.shared.columns(kind, caps)
    }

    fn cell<'a>(&self, object: &'a StoreObject, column: &ColumnId, now: Timestamp) -> Cell<'a> {
        match &self.metrics {
            Some(metrics) => self
                .shared
                .cell_with(object, column, Some(metrics.as_ref()), now),
            None => self.shared.cell(object, column, now),
        }
    }

    fn rank_epoch(&self, column: &ColumnId) -> u64 {
        match &self.metrics {
            Some(metrics) if defs::is_usage(column.as_str()) => metrics.epoch(),
            Some(_) => 0,
            None => self.shared.rank_epoch(column),
        }
    }
}
