//! Workloads: Pods, Deployments, DaemonSets, StatefulSets, ReplicaSets, ReplicationControllers,
//! Jobs, CronJobs. Jobs and CronJobs have their own modules (`kinds::job`, `kinds::cronjob`,
//! E12-S08), which register them.

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
    crate::kinds::job::init(cx);
    crate::kinds::cronjob::init(cx);
}
