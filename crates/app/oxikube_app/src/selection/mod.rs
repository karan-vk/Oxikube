//! Bulk actions on a selection (E12-S19): delete, restart, and edit labels or annotations of
//! several rows at once, on the E07 multi-select framework.
//!
//! Each is one guarded [`Command`](oxikube_domain::command::Command): `selection::Delete`,
//! `selection::Restart` and `selection::Label` (one command with a labels / annotations switch,
//! not two). The palette, the keys, the context menu and a hosted agent all send the same command.
//!
//! | File | Holds |
//! |---|---|
//! | `commands` | [`register_commands`]: the three handlers; [`MAX_BATCH`] |
//! | `ops` | the work on one object: delete, restart, label patch |
//! | `rows` | one object's audit record around its work |
//! | `report` | [`BulkReport`], [`BulkRow`], [`BulkStatus`]: the result per object |
//! | `flow` | [`SelectionFlow`], [`SelectionPlan`]: what a dialog asks, and the run that answers the guard |
//!
//! # The guard, once for the whole batch
//!
//! * **Read-only** blocks the batch before the first request, for every initiator, like any
//!   mutation. The permit's writer re-reads the flag before every request too, so a batch that
//!   outlives the check stops (its remaining rows fail, they are not skipped silently).
//! * **One confirmation, at the highest tier among the objects**: [`Command::effective_risk`]
//!   takes the maximum over the targets (a Namespace among a hundred Pods makes the batch
//!   `irreversible`), and the guard maps that to a tier. The size of the selection never lowers
//!   it. A type-the-name tier asks for the cluster's context name, not every object's name (a
//!   selection has no single name, and a long list of names cannot be typed): the same phrase
//!   the E07 delete dialog asks for several objects.
//! * **Per-object audit**: the command's own record names the cluster and the counts
//!   (`items=5 kinds=Pod:3,ConfigMap:2`); each object then gets a record of its own
//!   (`pod::Delete`, `resource::Delete`, `workload::Restart` or `selection::Label`) with the
//!   command's initiator and `batch=<id> via=selection::...`. Label and annotation values are
//!   never written, only how many keys are set and removed.
//! * **Bounded concurrency** ([`CONCURRENCY`](crate::workloads) at a time) through the batch runner
//!   `pod::Sanitize` introduced; a failure never stops the others; results keep the order asked.
//!
//! [`Command::effective_risk`]: oxikube_domain::command::Command::effective_risk

mod commands;
mod flow;
mod ops;
mod report;
mod rows;

#[cfg(test)]
mod tests;

pub use commands::{MAX_BATCH, SELECTION_COMMANDS, register_commands};
pub use flow::{SelectionError, SelectionFlow, SelectionPlan};
pub use report::{BulkReport, BulkRow, BulkStatus};
