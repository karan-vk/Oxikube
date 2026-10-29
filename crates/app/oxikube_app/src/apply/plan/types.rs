//! What a multi-object apply takes ([`PlanTarget`], [`ClusterPlan`], [`ApplyOptions`]) and gives
//! ([`ObjectResult`], [`ObjectStatus`], [`PlanReport`]).

use oxikube_domain::apply::ApplyPlan;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_ports::CancellationToken;
use serde::{Deserialize, Serialize};

use crate::apply::types::{ApplyMethod, FieldConflict};

/// Where a plan goes: the cluster, and the namespace a namespaced object that names none is put
/// in (`None` is `default`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanTarget {
    /// The cluster to apply to.
    pub cluster: ClusterId,
    /// The default namespace; `None` is `default`.
    pub namespace: Option<String>,
}

/// An [`ApplyPlan`] for one cluster: what [`ApplyService::plan`](super::super::ApplyService::plan)
/// builds and [`dry_run`](super::super::ApplyService::dry_run_plan) and
/// [`apply`](super::super::ApplyService::apply_plan) take. Each [`ObjectResult::index`] is an index
/// into [`plan`](Self::plan)`.items`, whose `doc` keeps the file and line the object came from.
///
/// The objects may be Secrets: the `Debug` form names kinds, never fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterPlan {
    /// Where it goes.
    pub target: PlanTarget,
    /// The objects, sanitised, in kind order, with their edges.
    pub plan: ApplyPlan,
}

/// How to apply a plan. Everything off by default: no force, a failure skips only what needs it
/// (as `kubectl apply`), the field manager `oxikube`.
#[derive(Debug, Clone, Default)]
pub struct ApplyOptions {
    /// Take over the fields other field managers own. Only for a run the user chose it for, after
    /// seeing the conflicts.
    pub force_conflicts: bool,
    /// Stop at the first failure: a failed dry run applies nothing, and a failed apply skips every
    /// later rank (what was already applied stays).
    pub stop_on_error: bool,
    /// The server-side apply field manager; `None` is `oxikube`.
    pub field_manager: Option<String>,
    /// Stops the run between objects: what is left is reported
    /// [`Skipped`](ObjectStatus::Skipped). Dropping the stream stops it at once instead.
    pub cancel: CancellationToken,
}

/// What happened to one object, or, in a dry run, what applying it would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ObjectStatus {
    /// The object did not exist: it was (would be) made.
    Created,
    /// The object existed and was (would be) changed.
    Configured,
    /// The object existed and stayed (would stay) as it was.
    Unchanged,
    /// Dry run only: the object needs a Namespace or CustomResourceDefinition the plan creates,
    /// so the server cannot rehearse it yet. It is rehearsed again, and applied, once that object
    /// has been applied.
    DeferredDryRun,
    /// Dry run only: the server cannot dry-run this kind. It is applied without a preview.
    NoPreviewAvailable,
    /// Refused by the server (or the guard's re-check); nothing was written for it.
    Failed {
        /// The server's message, names and fields only.
        reason: String,
        /// The fields the server rejected, in its notation (`spec.replicas`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<String>,
        /// Fields other field managers own: apply again with
        /// [`force_conflicts`](ApplyOptions::force_conflicts) to take them over.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        conflicts: Vec<FieldConflict>,
    },
    /// Not sent: an object it needs did not apply, the run stopped (stop on error, cancel), or
    /// its own dry run could not be judged. The reason names which.
    Skipped {
        /// Why, naming the object it needed.
        reason: String,
    },
}

impl ObjectStatus {
    /// `kubectl apply`'s word: "created", "configured", "unchanged", plus "deferred",
    /// "no preview", "failed", "skipped".
    pub fn word(&self) -> &'static str {
        match self {
            ObjectStatus::Created => "created",
            ObjectStatus::Configured => "configured",
            ObjectStatus::Unchanged => "unchanged",
            ObjectStatus::DeferredDryRun => "deferred",
            ObjectStatus::NoPreviewAvailable => "no preview",
            ObjectStatus::Failed { .. } => "failed",
            ObjectStatus::Skipped { .. } => "skipped",
        }
    }

    /// Whether the object did not (will not) reach the cluster.
    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            ObjectStatus::Failed { .. } | ObjectStatus::Skipped { .. }
        )
    }

    pub(super) fn skipped(reason: impl Into<String>) -> Self {
        ObjectStatus::Skipped {
            reason: reason.into(),
        }
    }
}

/// The result for one object of a plan. Names, never a body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectResult {
    /// Its index in the plan's items (its place in apply order, from 0).
    pub index: usize,
    /// "ConfigMap shop/web", or "Object 3" for one that names nothing.
    pub label: String,
    /// The object, when it names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<ResourceRef>,
    /// Whether this is the dry run's answer.
    pub dry_run: bool,
    /// What happened.
    #[serde(flatten)]
    pub status: ObjectStatus,
    /// How it was (would be) sent; a fallback means the server cannot server-side apply the kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<ApplyMethod>,
}

/// Every object's result, in plan order: the aggregate of a dry run or an apply.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlanReport {
    /// One per object of the plan, by [`ObjectResult::index`].
    pub objects: Vec<ObjectResult>,
}

impl PlanReport {
    /// How many objects have the status `word` ([`ObjectStatus::word`]).
    pub fn count(&self, word: &str) -> usize {
        self.objects
            .iter()
            .filter(|object| object.status.word() == word)
            .count()
    }

    /// How many objects failed or were skipped.
    pub fn failures(&self) -> usize {
        self.objects
            .iter()
            .filter(|object| object.status.is_failure())
            .count()
    }

    /// "2 created, 1 configured, 1 failed": the non-zero counts, in [`ObjectStatus::word`] order.
    pub fn summary(&self) -> String {
        let words = [
            "created",
            "configured",
            "unchanged",
            "deferred",
            "no preview",
            "failed",
            "skipped",
        ];
        let parts: Vec<String> = words
            .iter()
            .filter_map(|word| match self.count(word) {
                0 => None,
                n => Some(format!("{n} {word}")),
            })
            .collect();
        if parts.is_empty() {
            "nothing to apply".to_owned()
        } else {
            parts.join(", ")
        }
    }

    pub(super) fn sort(&mut self) {
        self.objects.sort_by_key(|object| object.index);
    }
}
