//! Rollout history and rollback (E12-S06): the pure half of `kubectl rollout history|undo`.
//!
//! A workload keeps its own revisions, and reading them is a function of the objects only:
//!
//! * a **Deployment** keeps one ReplicaSet per pod template it rolled out, each with the revision
//!   in `deployment.kubernetes.io/revision` and a copy of the template;
//! * a **StatefulSet** or **DaemonSet** keeps `ControllerRevision` objects (`apps/v1`) with a
//!   `revision` number and the strategic merge patch that restores the template in `data`.
//!
//! This module turns those objects into [`Revision`]s for a history table and into a
//! [`RollbackPlan`]: the patch that restores a chosen revision, built the way kubectl builds it.
//! It does no I/O: `oxikube_app`'s `RolloutService` lists the objects through the ports, calls
//! these functions and sends the patch through the `MutationGuard`; the kube adapter's
//! `rollout_undo` shares the Deployment half.
//!
//! | File | Holds |
//! |---|---|
//! | `selector` | [`label_selector`] (a workload's `spec.selector` as a list selector) and the ownership check |
//! | `template` | the pod template helpers: images, normalising before a comparison, [`TemplateChange`] |
//! | `deployment` | [`owned_replicasets`], [`deployment_history`], [`plan_deployment_rollback`] |
//! | `controller` | [`owned_controller_revisions`], [`controller_history`], [`plan_controller_rollback`] for StatefulSets and DaemonSets |
//!
//! # The algorithm
//!
//! The target is the revision asked for, or by default the previous one (the second newest). A
//! Deployment is patched with an RFC 6902 patch that **replaces `spec.template`** with the target
//! ReplicaSet's template minus its `pod-template-hash` label (a merge patch could not remove
//! fields the old template lacks), and lays the ReplicaSet's annotations over the Deployment's
//! own bookkeeping ones, which carries the change cause back. A StatefulSet or DaemonSet is
//! patched with the ControllerRevision's `data`, a strategic merge patch whose template carries
//! `$patch: replace`. A paused Deployment is refused with kubectl's own message, and a target
//! whose template is the one already running changes nothing ([`PlanAction::AlreadyCurrent`]).

mod controller;
mod deployment;
mod selector;
mod template;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use jiff::Timestamp;
use serde_json::Value;

use crate::ids::Gvk;
use crate::{OxiError, OxiResult, Resource};

pub use controller::{controller_history, owned_controller_revisions, plan_controller_rollback};
pub use deployment::{
    deployment_history, owned_replicasets, plan_deployment_rollback, refuse_if_paused, revision_of,
};
pub use selector::label_selector;
pub use template::{MAX_CHANGES, TemplateChange, images_of};

/// The annotation holding a revision number, on a Deployment (the current one) and on each of its
/// ReplicaSets.
pub const REVISION_ANNOTATION: &str = "deployment.kubernetes.io/revision";

/// The annotation `kubectl annotate` / `--record` use to say why a revision exists.
pub const CHANGE_CAUSE_ANNOTATION: &str = "kubernetes.io/change-cause";

/// The label the Deployment controller adds to a ReplicaSet and its pods to tell its ReplicaSets
/// apart; never part of the Deployment's own template.
pub const HASH_LABEL: &str = "pod-template-hash";

/// The controllers that keep a revision history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RolloutKind {
    /// `apps/v1` Deployment: the history is its ReplicaSets.
    Deployment,
    /// `apps/v1` StatefulSet: the history is its ControllerRevisions.
    StatefulSet,
    /// `apps/v1` DaemonSet: the history is its ControllerRevisions.
    DaemonSet,
}

impl RolloutKind {
    /// The kind `gvk` names, `None` for any other (a ReplicaSet has no history of its own: its
    /// owning Deployment has).
    pub fn of(gvk: &Gvk) -> Option<Self> {
        match (&*gvk.group, &*gvk.kind) {
            ("apps", "Deployment") => Some(Self::Deployment),
            ("apps", "StatefulSet") => Some(Self::StatefulSet),
            ("apps", "DaemonSet") => Some(Self::DaemonSet),
            _ => None,
        }
    }

    /// The Kubernetes kind name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deployment => "Deployment",
            Self::StatefulSet => "StatefulSet",
            Self::DaemonSet => "DaemonSet",
        }
    }

    /// The lower-case name kubectl prints (`deployment`, `statefulset`, `daemonset`).
    pub const fn kubectl_name(self) -> &'static str {
        match self {
            Self::Deployment => "deployment",
            Self::StatefulSet => "statefulset",
            Self::DaemonSet => "daemonset",
        }
    }
}

/// Replicas a revision's ReplicaSet runs; a ControllerRevision has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevisionReplicas {
    /// `spec.replicas` of the ReplicaSet.
    pub desired: u32,
    /// `status.readyReplicas` of the ReplicaSet.
    pub ready: u32,
}

/// One revision of a workload's rollout history, as a table row.
#[derive(Debug, Clone, PartialEq)]
pub struct Revision {
    /// The revision number.
    pub revision: i64,
    /// The object that holds it: a ReplicaSet's or a ControllerRevision's name.
    pub name: Arc<str>,
    /// The `kubernetes.io/change-cause` annotation, when one was recorded and is not empty.
    pub change_cause: Option<String>,
    /// The images of the template's containers, in order.
    pub images: Vec<String>,
    /// The ReplicaSet's replicas (a Deployment's revisions only).
    pub replicas: Option<RevisionReplicas>,
    /// When the revision's object was created.
    pub created: Option<Timestamp>,
    /// Whether this is the revision the workload runs now.
    pub current: bool,
    /// How this revision's template differs from the running one (empty for the current one).
    /// At most [`MAX_CHANGES`], the "preview" a rollback dialog shows before it asks.
    pub changes: Vec<TemplateChange>,
}

/// How a patch is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchFlavor {
    /// RFC 6902, `application/json-patch+json`: the body is the array of operations.
    Json,
    /// Strategic merge patch, `application/strategic-merge-patch+json`.
    Strategic,
}

/// What a rollback does.
#[derive(Debug, Clone, PartialEq)]
pub enum PlanAction {
    /// The workload already runs the target's template: nothing is sent.
    AlreadyCurrent,
    /// Send this patch to the workload.
    Patch {
        /// How the body is interpreted.
        flavor: PatchFlavor,
        /// The patch document.
        body: Value,
    },
}

/// A rollback worked out from the objects: where from, where to, and the patch.
#[derive(Debug, Clone, PartialEq)]
pub struct RollbackPlan {
    /// The workload's kind.
    pub kind: RolloutKind,
    /// The revision the workload runs now, when it can be told.
    pub from_revision: Option<i64>,
    /// The revision being restored.
    pub to_revision: i64,
    /// What to send.
    pub action: PlanAction,
}

/// The `kubernetes.io/change-cause` of `object`, when one was recorded and is not empty.
fn change_cause(object: &Resource) -> Option<String> {
    object
        .meta
        .annotations
        .get(CHANGE_CAUSE_ANNOTATION)
        .filter(|cause| !cause.is_empty())
        .map(|cause| cause.to_string())
}

/// Rejects a revision below 1 before anything is looked up.
fn check_revision(to_revision: Option<i64>) -> OxiResult<()> {
    if to_revision.is_some_and(|revision| revision < 1) {
        return Err(OxiError::validation("a revision is a number from 1"));
    }
    Ok(())
}

/// The entry of `owned` (oldest revision first) a rollback restores: the one at `wanted`, or by
/// default the previous one (the second newest).
fn find_revision(owned: &[(i64, Resource)], wanted: Option<i64>) -> Option<&(i64, Resource)> {
    match wanted {
        Some(wanted) => owned.iter().find(|(number, _)| *number == wanted),
        None => owned.len().checked_sub(2).and_then(|i| owned.get(i)),
    }
}
