//! A Deployment's history (its ReplicaSets) and the rollback to one of them.

use serde_json::{Map, Value};

use super::selector::is_controlled_by;
use super::template::{changes, images_of, normalised, running_template};
use super::{
    PatchFlavor, PlanAction, REVISION_ANNOTATION, Revision, RevisionReplicas, RollbackPlan,
    RolloutKind, change_cause, check_revision, find_revision,
};
use crate::json::JsonRef;
use crate::{OxiError, OxiResult, Resource};

/// Bookkeeping annotations: a ReplicaSet's are not copied back to the Deployment and the
/// Deployment keeps its own (kubectl's `annotationsToSkip`).
const SKIPPED_ANNOTATIONS: [&str; 6] = [
    "kubectl.kubernetes.io/last-applied-configuration",
    REVISION_ANNOTATION,
    "deployment.kubernetes.io/revision-history",
    "deployment.kubernetes.io/desired-replicas",
    "deployment.kubernetes.io/max-replicas",
    "deprecated.deployment.rollback.to",
];

/// A revision number: the annotation parsed, `None` when absent or not a number.
pub fn revision_of(object: &Resource) -> Option<i64> {
    object
        .meta
        .annotations
        .get(REVISION_ANNOTATION)?
        .parse()
        .ok()
}

/// The ReplicaSets among `candidates` that `deployment` controls, each with its revision, oldest
/// revision first. ReplicaSets without a readable revision are not part of the history.
pub fn owned_replicasets(
    deployment: &Resource,
    candidates: impl IntoIterator<Item = Resource>,
) -> Vec<(i64, Resource)> {
    let mut found: Vec<(i64, Resource)> = candidates
        .into_iter()
        .filter(|rs| is_controlled_by(rs, deployment))
        .filter_map(|rs| Some((revision_of(&rs)?, rs)))
        .collect();
    found.sort_by_key(|(revision, _)| *revision);
    found
}

/// The rollout history of `deployment` from its [`owned_replicasets`]: newest revision first, the
/// Deployment's current one flagged, each with how its template differs from the running one.
pub fn deployment_history(deployment: &Resource, owned: &[(i64, Resource)]) -> Vec<Revision> {
    let current = revision_of(deployment);
    let running = running_template(deployment);
    owned
        .iter()
        .rev()
        .map(|(revision, rs)| {
            let is_current = current == Some(*revision);
            let template = rs.get("/spec/template");
            let changes = match (&running, template, is_current) {
                (Some(running), Some(template), false) => {
                    changes(running, &normalised(template.to_value()))
                }
                _ => Vec::new(),
            };
            Revision {
                revision: *revision,
                name: rs.name().into(),
                change_cause: change_cause(rs),
                images: template.map(images_of).unwrap_or_default(),
                replicas: Some(RevisionReplicas {
                    desired: count(rs, "/spec/replicas"),
                    ready: count(rs, "/status/readyReplicas"),
                }),
                created: rs.meta.creation,
                current: is_current,
                changes,
            }
        })
        .collect()
}

fn count(object: &Resource, pointer: &str) -> u32 {
    object
        .get_i64(pointer)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
}

/// Refuses a paused Deployment with kubectl's own sentence, before anything else is read.
///
/// # Errors
///
/// `Conflict` when `spec.paused` is true.
pub fn refuse_if_paused(deployment: &Resource) -> OxiResult<()> {
    if deployment.get_bool("/spec/paused") == Some(true) {
        return Err(OxiError::conflict(format!(
            "you cannot rollback a paused deployment; resume it first with 'kubectl rollout \
             resume deployment/{}' and try again",
            deployment.name()
        )));
    }
    Ok(())
}

/// Works out `kubectl rollout undo` for `deployment`: restore `to_revision`, or by default the
/// previous revision (the second newest).
///
/// # Errors
///
/// * `Validation` for a `to_revision` below 1, a revision not in the history, and a Deployment
///   with no earlier revision to go back to;
/// * `Conflict` for a paused Deployment, with kubectl's message.
pub fn plan_deployment_rollback(
    deployment: &Resource,
    owned: &[(i64, Resource)],
    to_revision: Option<i64>,
) -> OxiResult<RollbackPlan> {
    check_revision(to_revision)?;
    refuse_if_paused(deployment)?;
    let (to, target) = choose(owned, to_revision, deployment.name())?;
    let template = restored_template(target)?;
    let action = if same_template(&template, deployment.get("/spec/template")) {
        PlanAction::AlreadyCurrent
    } else {
        PlanAction::Patch {
            flavor: PatchFlavor::Json,
            body: serde_json::json!([
                {"op": "replace", "path": "/spec/template", "value": template},
                {"op": "add", "path": "/metadata/annotations",
                 "value": restored_annotations(deployment, target)},
            ]),
        }
    };
    Ok(RollbackPlan {
        kind: RolloutKind::Deployment,
        from_revision: revision_of(deployment),
        to_revision: to,
        action,
    })
}

/// The ReplicaSet to restore: the one at `wanted`, or the second newest.
fn choose<'a>(
    history: &'a [(i64, Resource)],
    wanted: Option<i64>,
    deployment: &str,
) -> OxiResult<(i64, &'a Resource)> {
    match (find_revision(history, wanted), wanted) {
        (Some((revision, rs)), _) => Ok((*revision, rs)),
        (None, Some(revision)) => Err(OxiError::validation(format!(
            "unable to find revision {revision} of deployment {deployment} in its history"
        ))),
        (None, None) => Err(OxiError::validation(format!(
            "no rollout history found for deployment {deployment}"
        ))),
    }
}

/// The ReplicaSet's pod template without the hash label the controller added.
fn restored_template(rs: &Resource) -> OxiResult<Value> {
    rs.get("/spec/template")
        .filter(|t| t.is_object())
        .map(|t| strip_hash(t.to_value()))
        .ok_or_else(|| {
            OxiError::validation(format!("replicaset {} has no pod template", rs.name()))
        })
}

fn strip_hash(mut template: Value) -> Value {
    if let Some(labels) = template
        .pointer_mut("/metadata/labels")
        .and_then(Value::as_object_mut)
    {
        labels.shift_remove(super::HASH_LABEL);
    }
    template
}

/// Whether the Deployment already runs `restored` (both compared without the hash label).
fn same_template(restored: &Value, current: Option<JsonRef<'_>>) -> bool {
    current.is_some_and(|current| normalised(current.to_value()) == normalised(restored.clone()))
}

/// The Deployment's annotations after the undo, as kubectl computes them: the Deployment's own
/// bookkeeping annotations (revision, last-applied configuration, ...) stay, and the ReplicaSet's
/// other annotations are laid over them.
fn restored_annotations(deployment: &Resource, rs: &Resource) -> Value {
    let skipped = |key: &str| SKIPPED_ANNOTATIONS.contains(&key);
    let own = deployment
        .meta
        .annotations
        .iter()
        .filter(|(key, _)| skipped(key));
    let restored = rs.meta.annotations.iter().filter(|(key, _)| !skipped(key));
    let merged: Map<String, Value> = own
        .chain(restored)
        .map(|(key, value)| (key.to_string(), Value::String(value.to_string())))
        .collect();
    Value::Object(merged)
}
