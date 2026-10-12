//! [`Mirror`]: a subscription's rows as the feed keeps them, to know which object a `Remove`
//! removed, and [`RowTally`], what a tally does with each change.

use std::sync::Arc;

use super::nodes::NodeTally;
use super::pods::PodTally;
use super::snapshot::FeedStatus;
use crate::store::{RowChange, RowOp, StoreDelta, StoreObject};

/// A tally that follows a subscription row by row.
pub(crate) trait RowTally {
    /// The rows were replaced by `rows`: count them from scratch.
    fn reset(&mut self, rows: &[Arc<StoreObject>]);
    /// `object` arrived or changed.
    fn upsert(&mut self, object: &StoreObject);
    /// `object` left.
    fn remove(&mut self, object: &StoreObject);
}

impl RowTally for PodTally {
    fn reset(&mut self, rows: &[Arc<StoreObject>]) {
        self.clear();
        for row in rows {
            RowTally::upsert(self, row);
        }
    }

    fn upsert(&mut self, object: &StoreObject) {
        if let Some(pod) = object.resource() {
            PodTally::upsert(self, object.key(), pod);
        }
    }

    fn remove(&mut self, object: &StoreObject) {
        PodTally::remove(self, &object.key());
    }
}

impl RowTally for NodeTally {
    fn reset(&mut self, rows: &[Arc<StoreObject>]) {
        self.clear();
        for row in rows {
            RowTally::upsert(self, row);
        }
    }

    fn upsert(&mut self, object: &StoreObject) {
        if let Some(node) = object.resource() {
            NodeTally::upsert(self, node);
        }
    }

    fn remove(&mut self, object: &StoreObject) {
        NodeTally::remove(self, object.name());
    }
}

/// A subscription's rows and status, kept in step with its deltas.
#[derive(Default)]
pub(crate) struct Mirror {
    rows: Vec<Arc<StoreObject>>,
    status: FeedStatus,
}

impl Mirror {
    /// The rows, in the subscription's order.
    pub fn rows(&self) -> &[Arc<StoreObject>] {
        &self.rows
    }

    /// The feed's status after the last delta.
    pub fn status(&self) -> &FeedStatus {
        &self.status
    }

    /// Applies `delta` to the rows and tells `tally` what changed.
    pub fn apply(&mut self, delta: StoreDelta, partial: bool, tally: &mut impl RowTally) {
        match delta.rows {
            RowChange::Snapshot(rows) => {
                self.rows = rows;
                tally.reset(&self.rows);
            }
            RowChange::Ops(ops) => {
                for op in ops {
                    self.apply_op(op, tally);
                }
            }
            RowChange::Unchanged => {}
        }
        self.status = FeedStatus {
            state: delta.state,
            partial,
        };
    }

    fn apply_op(&mut self, op: RowOp, tally: &mut impl RowTally) {
        match op {
            RowOp::Insert { index, object } if index <= self.rows.len() => {
                tally.upsert(&object);
                self.rows.insert(index, object);
            }
            RowOp::Update { index, object } if index < self.rows.len() => {
                let old = std::mem::replace(&mut self.rows[index], object);
                if old.key() != self.rows[index].key() {
                    tally.remove(&old);
                }
                tally.upsert(&self.rows[index]);
            }
            RowOp::Remove { index } if index < self.rows.len() => {
                tally.remove(&self.rows.remove(index));
            }
            // Out of step (ops are ordered, so this does not happen); the next snapshot repairs it.
            RowOp::Insert { .. } | RowOp::Update { .. } | RowOp::Remove { .. } => {}
        }
    }
}
