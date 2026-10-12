//! `rollout history`: the Deployment's revisions.

use oxikube_domain::OxiResult;
use oxikube_domain::rollout::deployment_history;
use oxikube_ports::ResourcePort;
use tracing::debug;

use super::RolloutRevision;
use super::revisions::revisions;
use crate::algorithms::api::deployment_gvk;

/// The rollout history of the Deployment `namespace/name`, oldest revision first.
///
/// One entry per ReplicaSet the Deployment controls that carries a revision, so the length is
/// bounded by the Deployment's `revisionHistoryLimit` (plus the current one). The current
/// revision is flagged.
///
/// # Errors
///
/// `NotFound` for a missing Deployment, and the port's errors for the reads.
pub async fn rollout_history(
    port: &dyn ResourcePort,
    namespace: &str,
    name: &str,
) -> OxiResult<Vec<RolloutRevision>> {
    let deployment = port.get(&deployment_gvk(), Some(namespace), name).await?;
    debug!(op = "rollout_history", namespace, name, "algorithm");
    let history = revisions(port, &deployment).await?;
    // The domain lists newest first; this API has always been oldest first.
    Ok(deployment_history(&deployment, &history)
        .into_iter()
        .rev()
        .map(|revision| RolloutRevision {
            revision: revision.revision,
            replica_set: revision.name.to_string(),
            change_cause: revision.change_cause,
            images: revision.images,
            created: revision.created,
            current: revision.current,
        })
        .collect())
}
