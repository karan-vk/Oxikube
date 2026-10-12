//! Rollout history and rollback (E12-S06): `kubectl rollout history|undo` for Deployments,
//! StatefulSets and DaemonSets, as `workload::Rollback` on the `CommandBus`.
//!
//! The algorithm is pure and lives in [`oxikube_domain::rollout`] (shared with the kube adapter's
//! `rollout_undo`): which objects make the history, which revision is the previous one, the patch
//! that restores it. This module is the orchestration over the ports.
//!
//! | File | Holds |
//! |---|---|
//! | `service` | [`RolloutService`]: `history` (reads, on demand) and `rollback` (runs inside the guarded command) |
//! | `result` | [`RolloutHistory`] (what the history dialog shows) and [`RollbackResult`] |
//! | `command` | the `workload::Rollback` handler, installed by `register_rollback` |
//! | `runner` | [`RollbackRunner`]: dispatches `workload::Rollback` as the user for a dialog that already asked |
//!
//! | Command | Tool | Does |
//! |---|---|---|
//! | `workload::Rollback` | `k8s.workload_rollback` | restore a pod template from the workload's history; medium risk |
//!
//! # Where the history comes from
//!
//! * A **Deployment**: the ReplicaSets it controls (`ownerReferences`, by uid), each with its
//!   revision in `deployment.kubernetes.io/revision` and the `kubernetes.io/change-cause`
//!   annotation. The ReplicaSets are listed with the Deployment's own selector.
//! * A **StatefulSet** or **DaemonSet**: the `ControllerRevision` objects it owns (`revision`,
//!   `data`).
//! * A **ReplicaSet** is not a workload with a history, but it names a revision of the Deployment
//!   that owns it: a rollback "of a ReplicaSet" rolls that Deployment back to this ReplicaSet's
//!   revision.
//!
//! History is read on demand when a dialog opens (a list of the owned objects through the
//! cluster's reader); it starts no feed, so a ReplicaSets feed that is already running is not
//! duplicated.
//!
//! # Where the guard is
//!
//! `workload::Rollback` is a [`MutationGuard`](crate::MutationGuard) command: blocked on a
//! read-only cluster for every initiator, one confirmation (risk **medium**), a server dry run of
//! the patch before the real one, one audit record (`to_revision=` from the request, `from=` and
//! `to=` from the handler). The service writes through the guard's
//! [`Mutation`](crate::Mutation) only. The refusals are kubectl's: a paused Deployment, a revision
//! that was pruned by `revisionHistoryLimit`, a workload with no earlier revision. A target that
//! already runs the chosen template is reported ("already at revision N") and nothing is sent.

mod command;
mod result;
mod runner;
mod service;

#[cfg(test)]
mod tests;

pub(crate) use command::register_rollback;
pub use result::{RollbackOutcome, RollbackResult, RolloutHistory};
pub use runner::{RollbackReport, RollbackRunner};
pub use service::RolloutService;
