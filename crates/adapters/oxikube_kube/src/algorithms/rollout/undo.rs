//! `rollout undo`: put the Deployment's pod template back to an earlier revision.

use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::{Patch, ResourcePort, WriteOptions};
use serde_json::{Map, Value, json};
use tracing::debug;

use super::revisions::{HASH_LABEL, deployment_gvk, get_deployment, revision_of, revisions};

/// Annotations of a ReplicaSet that describe the ReplicaSet, not the Deployment, and are not
/// copied back (kubectl's `annotationsToSkip`).
const SKIPPED_ANNOTATIONS: [&str; 6] = [
    "kubectl.kubernetes.io/last-applied-configuration",
    "deployment.kubernetes.io/revision",
    "deployment.kubernetes.io/revision-history",
    "deployment.kubernetes.io/desired-replicas",
    "deployment.kubernetes.io/max-replicas",
    "deprecated.deployment.rollback.to",
];

/// The outcome of [`rollout_undo`].
#[derive(Debug, Clone, PartialEq)]
pub struct RolloutUndo {
    /// The Deployment's revision before the undo, if it had one.
    pub from_revision: Option<i64>,
    /// The revision whose template was restored.
    pub to_revision: i64,
    /// `true` when the Deployment already ran that template, so nothing was sent.
    pub unchanged: bool,
    /// The Deployment as the server returned it after the patch (with `options.dry_run`, as it
    /// would be stored); `None` when `unchanged`.
    pub deployment: Option<Resource>,
}

/// Rolls the Deployment `namespace/name` back to `to_revision`, or to the previous revision
/// (the second newest) when `None`.
///
/// With `options.dry_run` the server validates the patch and returns the Deployment it would
/// store.
///
/// # Errors
///
/// * `NotFound` for a missing Deployment;
/// * `Conflict` if the Deployment is paused (resume it first, as kubectl says);
/// * `Validation` for a `to_revision` below 1, for a revision not in the history, and for a
///   Deployment with no earlier revision to go back to;
/// * the port's errors for the reads and the patch.
pub async fn rollout_undo(
    port: &dyn ResourcePort,
    namespace: &str,
    name: &str,
    to_revision: Option<i64>,
    options: &WriteOptions,
) -> OxiResult<RolloutUndo> {
    if to_revision.is_some_and(|r| r < 1) {
        return Err(OxiError::validation("a revision is a number from 1"));
    }
    let deployment = get_deployment(port, namespace, name).await?;
    if deployment.get_bool("/spec/paused") == Some(true) {
        return Err(OxiError::conflict(format!(
            "deployment {name} is paused: resume it before rolling back"
        )));
    }
    let from_revision = revision_of(&deployment);
    let history = revisions(port, &deployment).await?;
    let (to_revision, target) = choose(&history, to_revision, name)?;

    let template = restored_template(target)?;
    if same_template(&template, deployment.get("/spec/template")) {
        return Ok(RolloutUndo {
            from_revision,
            to_revision,
            unchanged: true,
            deployment: None,
        });
    }
    debug!(
        op = "rollout_undo",
        namespace,
        name,
        to_revision,
        dry_run = options.dry_run,
        "algorithm"
    );
    let patch = Patch::json(json!([
        {"op": "replace", "path": "/spec/template", "value": template},
        {"op": "add", "path": "/metadata/annotations", "value": restored_annotations(target)},
    ]));
    let patched = port
        .patch(&deployment_gvk(), Some(namespace), name, &patch, options)
        .await?;
    Ok(RolloutUndo {
        from_revision,
        to_revision,
        unchanged: false,
        deployment: Some(patched),
    })
}

/// The ReplicaSet to restore: the one at `wanted`, or the second newest.
fn choose<'a>(
    history: &'a [(i64, Resource)],
    wanted: Option<i64>,
    deployment: &str,
) -> OxiResult<(i64, &'a Resource)> {
    // `history` is sorted by revision, oldest first.
    let found = match wanted {
        Some(revision) => history.iter().find(|(r, _)| *r == revision),
        None => history.len().checked_sub(2).and_then(|i| history.get(i)),
    };
    match (found, wanted) {
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
    let mut template = rs
        .get("/spec/template")
        .filter(|t| t.is_object())
        .cloned()
        .ok_or_else(|| {
            OxiError::validation(format!("replicaset {} has no pod template", rs.name()))
        })?;
    remove_hash(&mut template);
    Ok(template)
}

fn remove_hash(template: &mut Value) {
    if let Some(labels) = template
        .pointer_mut("/metadata/labels")
        .and_then(Value::as_object_mut)
    {
        labels.shift_remove(HASH_LABEL);
    }
}

/// Whether the Deployment already runs `restored` (both compared without the hash label).
fn same_template(restored: &Value, current: Option<&Value>) -> bool {
    current.is_some_and(|current| {
        let mut current = current.clone();
        remove_hash(&mut current);
        current == *restored
    })
}

/// The Deployment's annotations after the undo: the ReplicaSet's, less the bookkeeping ones.
fn restored_annotations(rs: &Resource) -> Value {
    let kept: Map<String, Value> = rs
        .meta
        .annotations
        .iter()
        .filter(|(key, _)| !SKIPPED_ANNOTATIONS.contains(&&***key))
        .map(|(key, value)| (key.to_string(), Value::String(value.to_string())))
        .collect();
    Value::Object(kept)
}
