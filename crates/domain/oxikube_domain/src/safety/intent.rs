//! [`MutationVerb`] and [`MutationIntent`]: what is about to change, free of any body.

use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::{Initiator, MutationFlags, Risk, classify};
use crate::audit::{AuditOutcome, AuditRecord};
use crate::ids::{ClusterId, Gvk, ResourceRef};

/// What a mutation does to its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationVerb {
    /// Create an object.
    Create,
    /// Replace an object.
    Update,
    /// Patch an object.
    Patch,
    /// Server-side apply a manifest.
    Apply,
    /// Delete one object.
    Delete,
    /// Delete every object matching a selector.
    DeleteCollection,
    /// Change a workload's replica count.
    Scale,
    /// Roll a workload's pods.
    Restart,
    /// Evict a pod through the eviction API.
    Evict,
    /// Mark a node unschedulable.
    Cordon,
    /// Mark a node schedulable.
    Uncordon,
    /// Cordon a node and evict its pods.
    Drain,
    /// Delete objects no longer in a desired set (for example a Helm or Argo CD prune).
    Prune,
}

impl MutationVerb {
    /// Every verb.
    pub const ALL: [MutationVerb; 13] = [
        MutationVerb::Create,
        MutationVerb::Update,
        MutationVerb::Patch,
        MutationVerb::Apply,
        MutationVerb::Delete,
        MutationVerb::DeleteCollection,
        MutationVerb::Scale,
        MutationVerb::Restart,
        MutationVerb::Evict,
        MutationVerb::Cordon,
        MutationVerb::Uncordon,
        MutationVerb::Drain,
        MutationVerb::Prune,
    ];

    /// Stable lowercase name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            MutationVerb::Create => "create",
            MutationVerb::Update => "update",
            MutationVerb::Patch => "patch",
            MutationVerb::Apply => "apply",
            MutationVerb::Delete => "delete",
            MutationVerb::DeleteCollection => "delete_collection",
            MutationVerb::Scale => "scale",
            MutationVerb::Restart => "restart",
            MutationVerb::Evict => "evict",
            MutationVerb::Cordon => "cordon",
            MutationVerb::Uncordon => "uncordon",
            MutationVerb::Drain => "drain",
            MutationVerb::Prune => "prune",
        }
    }
}

impl std::fmt::Display for MutationVerb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One mutation, described without its body, so it is safe to log, audit and show an agent.
///
/// A bulk operation is one intent: every item shares the [`batch_id`](Self::batch_id), and
/// [`count`](Self::count) is how many objects it covers (the `name` is the first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationIntent {
    /// The cluster it targets.
    pub cluster: ClusterId,
    /// The namespace, or `None` for a cluster-scoped object or a cluster-wide verb.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<Arc<str>>,
    /// The kind of the target.
    pub gvk: Gvk,
    /// The object name.
    pub name: Arc<str>,
    /// What it does.
    pub verb: MutationVerb,
    /// Its blast radius, from [`classify`].
    pub risk: Risk,
    /// Who asked.
    pub initiator: Initiator,
    /// Groups the items of one bulk operation; `None` for a single mutation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<Arc<str>>,
    /// How many objects a bulk operation covers; `None` for a single mutation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
}

impl MutationIntent {
    /// An intent on `target`, with its risk computed by [`classify`].
    pub fn new(
        target: &ResourceRef,
        verb: MutationVerb,
        flags: MutationFlags,
        initiator: Initiator,
    ) -> Self {
        Self {
            cluster: target.cluster.clone(),
            namespace: target.namespace.clone(),
            gvk: target.gvk.clone(),
            name: target.name.clone(),
            verb,
            risk: classify(verb, target.scope(), &target.gvk, flags),
            initiator,
            batch_id: None,
            count: None,
        }
    }

    /// Marks this as one bulk operation of `count` objects.
    #[must_use]
    pub fn in_batch(mut self, batch_id: &str, count: u32) -> Self {
        self.batch_id = Some(Arc::from(batch_id));
        self.count = Some(count);
        self
    }

    /// The object this intent names.
    pub fn target(&self) -> ResourceRef {
        match &self.namespace {
            Some(namespace) => ResourceRef::namespaced(
                self.cluster.clone(),
                self.gvk.clone(),
                namespace.clone(),
                self.name.clone(),
            ),
            None => ResourceRef::cluster_scoped(
                self.cluster.clone(),
                self.gvk.clone(),
                self.name.clone(),
            ),
        }
    }

    /// The audit record of this intent ending in `outcome`; carries the batch id.
    pub fn audit_record(
        &self,
        ts: Timestamp,
        who: &str,
        cmd: &str,
        dry_run: bool,
        outcome: AuditOutcome,
    ) -> AuditRecord {
        let record = AuditRecord::new(
            ts,
            who,
            self.initiator,
            cmd,
            self.target(),
            dry_run,
            outcome,
        );
        let record = record.with_verb(self.verb);
        match &self.batch_id {
            Some(batch_id) => record.with_batch_id(batch_id),
            None => record,
        }
    }
}

#[cfg(test)]
mod tests;
