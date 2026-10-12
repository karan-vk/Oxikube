//! Workload actions: trigger a CronJob now, suspend and resume CronJobs and Jobs (E12-S08),
//! scale the replica controllers (E12-S04), and restart and set the image of Deployments,
//! StatefulSets and DaemonSets (E12-S05).
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
//! | `batch` | `run_each` and `new_batch_id`: the per-object batch runner `pod::Sanitize` and the bulk `selection::*` commands share (bounded concurrency, results in order) |
//! | `service` | [`WorkloadService`]: `trigger_cronjob`, `suspend`, `resume` over the guard's [`Mutation`](crate::Mutation) |
//! | `commands` | [`register_commands`]: the `cronjob::Trigger`, `cronjob::Suspend`, `cronjob::Resume`, `job::Suspend`, `job::Resume` and `workload::Scale` handlers |
//! | `scale` | `WorkloadService::plan_scale` (reads the `scale` subresource and looks for an autoscaler) and `WorkloadService::scale` (runs inside the guarded command) |
//! | `scale_command` | the `workload::Scale` handler |
//! | `plan` | [`ScalePlan`] (what the dialog starts with), [`ScaleResult`], [`parse_replicas`], [`is_scalable`] |
//! | `runner` | [`ScaleRunner`]: dispatches `workload::Scale` on the bus as the user, answering its confirmation for a dialog that already asked; `run_confirmed`, the dispatch both runners share |
//! | `rollout` | `WorkloadService::restart`, `plan_set_image` and `set_image` (run inside the guarded commands) |
//! | `rollout_plan` | [`SetImagePlan`] (what the dialog starts with), [`RestartResult`], [`SetImageResult`], [`is_rollable`] |
//! | `rollout_commands` | the `workload::Restart` and `workload::SetImage` handlers |
//! | `set_image_runner` | [`SetImageRunner`]: dispatches `workload::SetImage` for the dialog, like [`ScaleRunner`] |
//!
//! | Command | Tool | Does |
//! |---|---|---|
//! | `workload::Scale` | `k8s.workload_scale` | patch `spec.replicas` through the `scale` subresource; low risk, medium for zero (see below) |
//! | `workload::Restart` | `k8s.workload_restart` | `kubectl rollout restart`: a merge patch stamping the pod template's `kubectl.kubernetes.io/restartedAt` (the kdash builder, time from the session clock); low risk |
//! | `workload::SetImage` | `k8s.workload_set_image` | `kubectl set image`: a strategic merge patch naming only the changed containers (init containers included); low risk |
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
//!
//! # Restart and set image: where the guard is
//!
//! Both are [`MutationGuard`](crate::MutationGuard) commands: blocked on a read-only cluster for
//! every initiator, one confirmation (risk **low**), a server dry run of the patch before the real
//! one, one audit record. They return when the server accepted the patch and never wait for the
//! rollout: the output asks a view to open the workload (`data.open`), whose panel shows the
//! updated, ready and available replicas as the controller reports them. The audit record carries
//! the server's `resourceVersion` (`rv=`); a restart adds the time it stamped (`restartedAt=`) and
//! a set image each container's old and new image (`web=nginx:1.26->nginx:1.27`).
//! A ReplicaSet and a ReplicationController have a pod template but never roll existing pods, so
//! they are refused ([`is_rollable`]).

mod batch;
mod commands;
mod plan;
mod pod_commands;
mod pods;
mod refs;
mod rollout;
mod rollout_commands;
mod rollout_plan;
mod runner;
mod sanitize;
mod scale;
mod scale_command;
mod service;
mod set_image_runner;

#[cfg(test)]
mod tests;

pub(crate) use batch::{CONCURRENCY, new_batch_id, run_each};
pub use commands::{WORKLOAD_COMMANDS, register_commands, register_pod_commands};
pub use plan::{MAX_REPLICAS, ScalePlan, ScaleResult, is_scalable, parse_replicas, replicas_word};
pub use pod_commands::POD_COMMANDS;
pub use pods::{Evicted, PodDeleted};
pub use rollout_plan::{RestartResult, SetImagePlan, SetImageResult, is_rollable};
pub use runner::{ScaleReport, ScaleRunner};
pub use sanitize::{MAX_SWEEP, PodResult, PodStatus, Sweep};
pub use service::{SuspendOutcome, Triggered, WorkloadService};
pub use set_image_runner::{SetImageReport, SetImageRunner};
