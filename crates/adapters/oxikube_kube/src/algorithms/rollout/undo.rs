//! `rollout undo`: put the Deployment's pod template back to an earlier revision. The choice of
//! the ReplicaSet and the patch are the domain's ([`plan_deployment_rollback`]); this sends it.

use oxikube_domain::rollout::{PlanAction, plan_deployment_rollback, refuse_if_paused};
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::{Patch, ResourcePort, WriteOptions};
use tracing::debug;

use super::revisions::revisions;
use crate::algorithms::api::deployment_gvk;

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
    let deployment = port.get(&deployment_gvk(), Some(namespace), name).await?;
    // Before the list: a paused Deployment costs one read, as it does for kubectl.
    refuse_if_paused(&deployment)?;
    let history = revisions(port, &deployment).await?;
    let plan = plan_deployment_rollback(&deployment, &history, to_revision)?;
    let PlanAction::Patch { body, .. } = plan.action else {
        return Ok(RolloutUndo {
            from_revision: plan.from_revision,
            to_revision: plan.to_revision,
            unchanged: true,
            deployment: None,
        });
    };
    debug!(
        op = "rollout_undo",
        namespace,
        name,
        to_revision = plan.to_revision,
        dry_run = options.dry_run,
        "algorithm"
    );
    let patched = port
        .patch(
            &deployment_gvk(),
            Some(namespace),
            name,
            &Patch::json(body),
            options,
        )
        .await?;
    Ok(RolloutUndo {
        from_revision: plan.from_revision,
        to_revision: plan.to_revision,
        unchanged: false,
        deployment: Some(patched),
    })
}
