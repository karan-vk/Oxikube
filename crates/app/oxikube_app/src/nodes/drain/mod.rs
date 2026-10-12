//! Node drain (E12-S10): the guarded `node::Drain` command, its per-pod audit records and the
//! progress the drain dialog follows.
//!
//! | File | Holds |
//! |---|---|
//! | `service` | [`NodeService`]: runs one drain through the guard's `Mutation` over the [`DrainPort`](oxikube_ports::DrainPort), audits each pod, publishes each step |
//! | `monitor` | [`DrainMonitor`]: who follows which running drain; one drain per node at a time |
//! | `command` | [`register_drain_command`]: the `node::Drain` handler (tool stub `k8s.node_drain`) |
//! | `flow` | [`DrainFlow`]: the dialog's side: read the node and its pods ([`DrainContext`], the plan before anything changes), start the drain with the typed name, follow it ([`DrainStart`]) |
//!
//! # Where the algorithm is
//!
//! `kubectl drain` itself (plan, cordon, evict a few pods at a time, back off while a
//! PodDisruptionBudget refuses, wait for each pod to go) is the kube adapter's
//! (`oxikube_kube::KubeDrain`, E04-S07) behind the `DrainPort`, per the plan review (#397); the
//! filters and the progress vocabulary are pure, in `oxikube_domain::drain`. This module is the
//! orchestration: the guard, the audit trail and the followers.
//!
//! # Guard and audit
//!
//! `node::Drain` is risk high: blocked on a read-only cluster before anything is read (and the
//! denial audited), a type-the-name confirmation, and the guard's record of the drain with the
//! flags (`ignore_daemonsets=true delete_emptydir_data=false force=false grace=pod
//! timeout=300s`), the batch id and, at the end, the counts. Each pod that goes (or fails) gets
//! its own record (`node::Drain` on the pod, `batch=<id> node=<node>`), so the drain and its
//! evictions read as one batch. The cordon and every eviction go through the guard's writer,
//! which re-checks read-only before each request. A dispatch dropped mid-way (the dialog's
//! Cancel) stops the drain and is audited as cancelled, with the batch id; the node stays
//! cordoned, and the dialog offers to uncordon it.
//!
//! # Plan first
//!
//! The dialog reads the node's pods ([`DrainFlow::open`]) and shows [`DrainContext::plan`] for the
//! chosen flags before anything changes: the pods to evict, those skipped (DaemonSet, mirror)
//! and those that would refuse the drain (unmanaged, emptyDir, DaemonSet without the flag). That
//! is this command's "server dry run": the adapter plans again from a fresh list, and again once
//! the node is cordoned.

mod command;
mod flow;
mod monitor;
mod service;

pub use command::register_drain_command;
pub use flow::{DrainContext, DrainFlow, DrainStart};
pub use monitor::DrainMonitor;
pub use service::{DrainReport, NodeService};
