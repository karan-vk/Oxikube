//! Workloads: Pods (registered by `kinds::pods`, rank 10), Deployments, DaemonSets,
//! StatefulSets, ReplicaSets, ReplicationControllers. Jobs and CronJobs have their own modules
//! (`kinds::job`, `kinds::cronjob`, E12-S08), which register them.

use gpui::App;
use oxikube_app::Section;

use super::{Builtin, generic, namespaced, register_from};

/// After Pods, which hold the first slot (rank 10).
const WORKLOADS: &[Builtin] = &[
    namespaced("apps", "v1", "Deployment", "deployments", "Deployments"),
    namespaced("apps", "v1", "DaemonSet", "daemonsets", "DaemonSets"),
    namespaced("apps", "v1", "StatefulSet", "statefulsets", "StatefulSets"),
    namespaced("apps", "v1", "ReplicaSet", "replicasets", "ReplicaSets"),
    namespaced(
        "",
        "v1",
        "ReplicationController",
        "replicationcontrollers",
        "Replication Controllers",
    ),
];

/// Registers the Workloads section's kinds other than Pods, from rank 20.
pub fn init(cx: &mut App) {
    register_from(cx, Section::WORKLOADS, WORKLOADS, 2, generic);
    crate::kinds::job::init(cx);
    crate::kinds::cronjob::init(cx);
}
