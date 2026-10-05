//! `rollout history`: the Deployment's revisions.

use oxikube_domain::OxiResult;
use oxikube_ports::ResourcePort;
use tracing::debug;

use super::RolloutRevision;
use super::revisions::{images, revision_of, revisions};
use crate::algorithms::api::deployment_gvk;

/// The annotation `kubectl annotate` / `--record` use to say why a revision exists.
const CHANGE_CAUSE: &str = "kubernetes.io/change-cause";

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
    let current = revision_of(&deployment);
    debug!(op = "rollout_history", namespace, name, "algorithm");
    let history = revisions(port, &deployment).await?;
    Ok(history
        .into_iter()
        .map(|(revision, rs)| RolloutRevision {
            revision,
            replica_set: rs.name().to_owned(),
            change_cause: rs
                .meta
                .annotations
                .get(CHANGE_CAUSE)
                .map(|cause| cause.to_string())
                .filter(|cause| !cause.is_empty()),
            images: rs.get("/spec/template").map(images).unwrap_or_default(),
            created: rs.meta.creation,
            current: current == Some(revision),
        })
        .collect())
}
