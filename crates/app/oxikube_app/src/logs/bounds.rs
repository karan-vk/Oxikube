//! The buffer bounds of the service: the default `logs.buffer_lines` and the per-cluster
//! overrides (`clusters.<id>.logs.buffer_lines`).
//!
//! Every session reads its bound from one shared cell at each commit, so a change is stored in
//! the cells first and then applied to the open buffers (a commit never writes an older bound
//! over a newer one). Each cluster that has had a session or an override has a cell of its own; sessions outside any
//! cluster share the default cell.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use oxikube_domain::ids::ClusterId;

use super::options::clamp_buffer_lines;

/// A bound that sessions read: the lines they may keep.
pub(super) type BoundCell = Arc<AtomicUsize>;

struct ClusterBound {
    cell: BoundCell,
    /// The cluster's own bound (clamped); `None` follows the default.
    own: Option<usize>,
}

/// The default bound and the clusters' own.
pub(super) struct Bounds {
    default: BoundCell,
    clusters: HashMap<ClusterId, ClusterBound>,
}

impl Bounds {
    pub(super) fn new(lines: usize) -> Self {
        Self {
            default: Arc::new(AtomicUsize::new(clamp_buffer_lines(lines))),
            clusters: HashMap::new(),
        }
    }

    /// The cell of sessions that belong to no cluster.
    pub(super) fn default_cell(&self) -> BoundCell {
        self.default.clone()
    }

    pub(super) fn default_lines(&self) -> usize {
        self.default.load(Ordering::Acquire)
    }

    /// The cell `cluster`'s sessions read, made on first use (at the default bound): a session
    /// opened before the cluster got an override of its own must still see it.
    pub(super) fn cell_for(&mut self, cluster: &ClusterId) -> BoundCell {
        let default = self.default_lines();
        self.clusters
            .entry(cluster.clone())
            .or_insert_with(|| ClusterBound {
                cell: Arc::new(AtomicUsize::new(default)),
                own: None,
            })
            .cell
            .clone()
    }

    pub(super) fn lines_for(&self, cluster: &ClusterId) -> usize {
        self.clusters.get(cluster).map_or_else(
            || self.default_lines(),
            |bound| bound.cell.load(Ordering::Acquire),
        )
    }

    /// Stores a new default (clamped), for the default cell and the clusters without an override.
    pub(super) fn set_default(&mut self, lines: usize) {
        let lines = clamp_buffer_lines(lines);
        self.default.store(lines, Ordering::Release);
        for bound in self.clusters.values().filter(|bound| bound.own.is_none()) {
            bound.cell.store(lines, Ordering::Release);
        }
    }

    /// Replaces the clusters' own bounds with `overrides` (clamped): a cluster missing from it
    /// goes back to the default.
    pub(super) fn set_overrides(
        &mut self,
        overrides: impl IntoIterator<Item = (ClusterId, usize)>,
    ) {
        let overrides: HashMap<ClusterId, usize> = overrides
            .into_iter()
            .map(|(id, lines)| (id, clamp_buffer_lines(lines)))
            .collect();
        // A cluster that lost its override keeps its cell (open sessions hold it), back at the
        // default.
        for (id, bound) in &mut self.clusters {
            bound.own = overrides.get(id).copied();
        }
        for (id, lines) in overrides {
            self.clusters
                .entry(id)
                .or_insert_with(|| ClusterBound {
                    cell: Arc::new(AtomicUsize::new(lines)),
                    own: Some(lines),
                })
                .own = Some(lines);
        }
        let default = self.default_lines();
        for bound in self.clusters.values() {
            bound
                .cell
                .store(bound.own.unwrap_or(default), Ordering::Release);
        }
    }
}
