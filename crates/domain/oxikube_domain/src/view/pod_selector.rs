//! [`pod_selector`]: the label selector of the pods a controller owns, for views that follow
//! those pods (the Metrics tab's per-pod lines).

use super::workload_detail::LabelSelector;
use super::{ViewError, check_kind, sub};
use crate::resource::Resource;

/// The selector of the pods of a Deployment, StatefulSet, DaemonSet, ReplicaSet or
/// ReplicationController.
///
/// Unlike [`WorkloadDetail::from_resource`](super::WorkloadDetail::from_resource) this accepts a
/// DaemonSet (whose `spec.selector` has the same shape), because it asks only which pods belong
/// to the controller, not for the replica-based panel.
///
/// # Errors
///
/// [`ViewError::WrongKind`] for any other kind.
pub fn pod_selector(res: &Resource) -> Result<LabelSelector, ViewError> {
    check_kind(
        res,
        "Deployment, StatefulSet, DaemonSet, ReplicaSet or ReplicationController",
        &[
            ("apps", "Deployment"),
            ("apps", "StatefulSet"),
            ("apps", "DaemonSet"),
            ("apps", "ReplicaSet"),
            ("", "ReplicationController"),
        ],
    )?;
    let plain = res.kind.kind.as_ref() == "ReplicationController";
    Ok(LabelSelector::from_json(
        sub(sub(res.json(), "spec"), "selector"),
        plain,
    ))
}
