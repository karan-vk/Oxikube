//! Workload actions (E12-S08): trigger a CronJob now, suspend and resume CronJobs and Jobs.
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
//! | `commands` | [`register_commands`]: the `cronjob::Trigger`, `cronjob::Suspend`, `cronjob::Resume`, `job::Suspend` and `job::Resume` handlers |
//!
//! | Command | Tool | Does |
//! |---|---|---|
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

mod commands;
mod service;

#[cfg(test)]
mod tests;

pub use commands::{WORKLOAD_COMMANDS, register_commands};
pub use service::{SuspendOutcome, Triggered, WorkloadService};
