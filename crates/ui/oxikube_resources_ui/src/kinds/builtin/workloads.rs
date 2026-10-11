//! Workloads: Pods, Deployments, DaemonSets, StatefulSets, ReplicaSets, ReplicationControllers,
//! Jobs, CronJobs.

use gpui::App;
use oxikube_app::Section;
use oxikube_domain::command::CommandId;

use super::{Builtin, namespaced, register};

const WORKLOADS: &[Builtin] = &[
    namespaced("", "v1", "Pod", "pods", "Pods"),
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
    namespaced("batch", "v1", "Job", "jobs", "Jobs"),
    namespaced("batch", "v1", "CronJob", "cronjobs", "CronJobs"),
];

/// A pod's own actions, as E08 and E09 ship them: logs, shell, attach, debug, port forward.
const POD_ACTIONS: &[CommandId] = &[
    CommandId::POD_VIEW_LOGS,
    CommandId::POD_SHELL,
    CommandId::POD_ATTACH,
    CommandId::POD_DEBUG,
    CommandId::POD_PORT_FORWARD,
];

/// Registers the Workloads section's kinds.
pub fn init(cx: &mut App) {
    register(cx, Section::WORKLOADS, WORKLOADS, |kind| {
        if kind.kind == "Pod" { POD_ACTIONS } else { &[] }
    });
}
