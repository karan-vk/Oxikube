//! Nodes (E12-S09): cordon and uncordon, the node table's columns, and the pods on a node.
//!
//! | File | Holds |
//! |---|---|
//! | `service` | [`set_unschedulable`]: cordon and uncordon, the `spec.unschedulable` patch through the guard's [`Mutation`](crate::Mutation) |
//! | `commands` | [`register_commands`]: the `node::Cordon` and `node::Uncordon` handlers |
//! | `columns` | [`NodeColumns`]: the node table's columns (Pods from the pods-by-node index, CPU% and Memory% as metrics slots), [`node_columns`], [`NodeUsageSource`] |
//! | `pods` | [`list_node_pods`]: the field-selector fallback behind the detail when the Pod feed is not active |
//!
//! # Cordon and uncordon
//!
//! Both are mutating commands, so they reach their handler only through the
//! [`MutationGuard`](crate::MutationGuard): a read-only cluster refuses them before any request
//! (and the denial is audited), a [`Risk::Low`](oxikube_domain::Risk::Low) confirmation is asked
//! ([`ConfirmTier::Simple`](oxikube_domain::ConfirmTier)), and one audit record names the node and
//! the [`Initiator`](oxikube_domain::Initiator). The handler sends a server-side dry run of the
//! patch first, then the patch itself (a dispatch that is itself a dry run stops after the dry
//! run). The patch is three lines: `{"spec": {"unschedulable": true}}` to cordon, and the same
//! with `null` to uncordon, which removes the field as `kubectl uncordon` does. Existing pods
//! stay where they are; moving them is drain (E12-S10).
//!
//! # Pods on a node
//!
//! The Pods column and the detail's allocation bars read the store's pods-by-node index
//! ([`NodeLoadsLease`](crate::store::NodeLoadsLease)), which follows the cluster-wide Pod feed
//! delta by delta in O(1). When that feed is not active (the budget refused it, or it is still
//! listing) the detail asks the API server for just this node's pods with
//! `spec.nodeName=<node>` ([`list_node_pods`]).

mod columns;
mod commands;
mod pods;
mod service;

#[cfg(test)]
mod tests;

pub use columns::{NodeColumns, NodeUsage, NodeUsageSource, node_columns};
pub use commands::register_commands;
pub use pods::{NodePods, list_node_pods};
pub use service::{NodeOutcome, set_unschedulable};
