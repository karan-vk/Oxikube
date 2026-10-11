//! Workload actions: trigger a CronJob now, suspend and resume CronJobs and Jobs (E12-S08), and
//! scale the replica controllers (E12-S04).
//!
//! Every action is a [`Command`](oxikube_domain::command::Command) on the `CommandBus`, so the
//! context menu, the `t` and `s` keys, the palette and a hosted agent run one behaviour; every one
//! mutates a cluster, so it goes through the [`MutationGuard`](crate::MutationGuard): blocked on a
//! read-only cluster for every initiator, a simple confirmation (risk `low`), a server dry run
//! before the real request, and one audit record. [`WorkloadService`] holds the logic;
//! [`register_commands`] installs the handlers.
//!
//! | File | Holds |
//! |---|---|
//! | `service` | [`WorkloadService`]: `trigger_cronjob`, `suspend`, `resume` over the guard's [`Mutation`](crate::Mutation) |
//! | `commands` | [`register_commands`]: the `cronjob::Trigger`, `cronjob::Suspend`, `cronjob::Resume`, `job::Suspend`, `job::Resume` and `workload::Scale` handlers |
//! | `scale` | `WorkloadService::plan_scale` (reads the `scale` subresource and looks for an autoscaler) and `WorkloadService::scale` (runs inside the guarded command) |
//! | `scale_command` | the `workload::Scale` handler |
//! | `plan` | [`ScalePlan`] (what the dialog starts with), [`ScaleResult`], [`parse_replicas`], [`is_scalable`] |
//! | `runner` | [`ScaleRunner`]: dispatches `workload::Scale` on the bus as the user, answering its confirmation for a dialog that already asked |
//!
//! | Command | Tool | Does |
//! |---|---|---|
//! | `workload::Scale` | `k8s.workload_scale` | patch `spec.replicas` through the `scale` subresource; low risk, medium for zero (see below) |
//! | `cronjob::Trigger` | `k8s.cronjob_trigger` | `kubectl create job --from=cronjob/x`: a Job from `spec.jobTemplate`, named by the server from `<cronjob>-manual-`, owned by the CronJob |
//! | `cronjob::Suspend`, `cronjob::Resume` | `k8s.cronjob_suspend`, `k8s.cronjob_resume` | merge patch of `spec.suspend` |
//! | `job::Suspend`, `job::Resume` | `k8s.job_suspend`, `k8s.job_resume` | the same on a Job; a Job that finished cannot be resumed |
//!
//! # The Job a trigger made
//!
//! The Job comes from [`oxikube_domain::build::job_from_cronjob`] (the kdash port `kubectl create
//! job --from` is built on, shared with the kube adapter's `trigger_cronjob`). Like kubectl it
//! ignores `spec.concurrencyPolicy`. The handler's output names the Job (`data.job`) and tells a
//! view where to go (`data.open`), and the audit record's detail says `job=<name>` next to the
//! initiator.
//!
//! The service reads no clock: a Job's age, duration and a CronJob's last schedule are computed by
//! the table from its own per-frame `now` (the `ClockPort` of the session), so tests that pass a
//! fixed `now` are deterministic.
//!
//! # Scale: where the guard is
//!
//! `workload::Scale` is a [`MutationGuard`](crate::MutationGuard) command: blocked on a read-only
//! cluster for every initiator, one confirmation, one audit record. Its risk is **low**; scaling
//! to zero raises it to **medium** (still one confirmation, but it says that every pod stops).
//! The service writes the `scale` subresource only (a merge patch of `spec.replicas`), never the
//! object. The audit record carries the requested count and, from the handler,
//! `before=` and `after=` ([`HandlerContext::note_audit`](crate::command_bus::HandlerContext::note_audit)).
//! A DaemonSet has no `scale` subresource: it is refused and not offered as an action.

mod commands;
mod plan;
mod runner;
mod scale;
mod scale_command;
mod service;

#[cfg(test)]
mod tests;

pub use commands::{WORKLOAD_COMMANDS, register_commands};
pub use plan::{MAX_REPLICAS, ScalePlan, ScaleResult, is_scalable, parse_replicas, replicas_word};
pub use runner::{ScaleReport, ScaleRunner};
pub use service::{SuspendOutcome, Triggered, WorkloadService};
