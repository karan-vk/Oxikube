//! Workloads: Pods (E12-S02, E12-S03), the replica controllers (E12-S04 to S06), DaemonSets
//! (E12-S07), Jobs and CronJobs (E12-S08).
//!
//! No seed puts a pod on the shared node except the Pod itself (the pause image, from the
//! pre-pulled list): the controllers have no replicas, the DaemonSet selects a node label nothing
//! has, the Job is suspended and the CronJob is suspended with a yearly schedule.

use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Scope;
use serde_json::{Value, json};

use super::{Api, KindCase, Seed};
use crate::images::PAUSE;

/// A namespaced built-in kind of the Workloads section.
const fn workload(
    group: &'static str,
    kind: &'static str,
    plural: &'static str,
    columns: &'static [&'static str],
    actions: &'static [CommandId],
    panel: bool,
    seed: fn(&Seed<'_>) -> Value,
) -> KindCase {
    KindCase {
        section: "workloads",
        plural,
        group,
        version: "v1",
        kind,
        scope: Scope::Namespaced,
        api: Api::BuiltIn,
        columns,
        actions,
        panel,
        seed,
    }
}

/// The rollout verbs (E12-S05) and rollback (E12-S06).
const ROLLING: &[CommandId] = &[
    CommandId::WORKLOAD_VIEW_LOGS,
    CommandId::WORKLOAD_SCALE,
    CommandId::WORKLOAD_RESTART,
    CommandId::WORKLOAD_SET_IMAGE,
    CommandId::WORKLOAD_ROLLBACK,
];

pub(super) const CASES: &[KindCase] = &[
    workload(
        "",
        "Pod",
        "pods",
        &[
            "Name",
            "Namespace",
            "Ready",
            "Status",
            "Restarts",
            "CPU",
            "Memory",
            "%CPU/R",
            "%CPU/L",
            "%MEM/R",
            "%MEM/L",
            "IP*",
            "Node*",
            "QoS*",
            "Service Account",
            "Controlled By",
            "Age",
            "Nominated Node*",
            "Last Restart*",
            "Labels*",
        ],
        &[
            CommandId::POD_JUMP_TO_OWNER,
            CommandId::POD_SHOW_NODE,
            CommandId::POD_VIEW_LOGS,
            CommandId::POD_SHELL,
            CommandId::POD_ATTACH,
            CommandId::POD_DEBUG,
            CommandId::POD_BROWSE_FILES,
            CommandId::POD_PORT_FORWARD,
            CommandId::POD_EVICT,
            CommandId::POD_DELETE,
            CommandId::POD_FORCE_DELETE,
            CommandId::POD_SANITIZE,
        ],
        true,
        pod,
    ),
    workload(
        "apps",
        "Deployment",
        "deployments",
        &[
            "Name",
            "Namespace",
            "Ready",
            "Up-to-date",
            "Available",
            "Replicas",
            "Age",
            "Conditions",
            "Images*",
            "Selector*",
            "Labels*",
        ],
        ROLLING,
        true,
        |_| controller("seed-deployment", json!({"replicas": 0})),
    ),
    workload(
        "apps",
        "DaemonSet",
        "daemonsets",
        &[
            "Name",
            "Namespace",
            "Desired",
            "Current",
            "Ready",
            "Up-to-date",
            "Available",
            "Node Selector",
            "Age",
            "Images*",
            "Labels*",
        ],
        &[
            CommandId::WORKLOAD_VIEW_LOGS,
            CommandId::WORKLOAD_RESTART,
            CommandId::WORKLOAD_SET_IMAGE,
            CommandId::WORKLOAD_ROLLBACK,
        ],
        true,
        daemonset,
    ),
    workload(
        "apps",
        "StatefulSet",
        "statefulsets",
        &[
            "Name",
            "Namespace",
            "Ready",
            "Replicas",
            "Age",
            "Service",
            "Conditions*",
            "Images*",
            "Labels*",
        ],
        ROLLING,
        true,
        |_| {
            controller(
                "seed-statefulset",
                json!({"replicas": 0, "serviceName": "seed-service"}),
            )
        },
    ),
    workload(
        "apps",
        "ReplicaSet",
        "replicasets",
        &[
            "Name",
            "Namespace",
            "Desired",
            "Current",
            "Ready",
            "Age",
            "Conditions*",
            "Images*",
            "Selector*",
            "Controlled By*",
            "Labels*",
        ],
        &[
            CommandId::WORKLOAD_VIEW_LOGS,
            CommandId::WORKLOAD_SCALE,
            CommandId::WORKLOAD_ROLLBACK,
        ],
        true,
        |_| controller("seed-replicaset", json!({"replicas": 0})),
    ),
    workload(
        "",
        "ReplicationController",
        "replicationcontrollers",
        &[
            "Name",
            "Namespace",
            "Replicas",
            "Desired",
            "Ready",
            "Selector",
            "Age",
            "Conditions*",
            "Images*",
            "Labels*",
        ],
        &[CommandId::WORKLOAD_VIEW_LOGS, CommandId::WORKLOAD_SCALE],
        true,
        replication_controller,
    ),
    workload(
        "batch",
        "Job",
        "jobs",
        &[
            "Name",
            "Namespace",
            "Status",
            "Completions",
            "Duration",
            "Conditions",
            "Age",
            "Labels*",
        ],
        &[
            CommandId::WORKLOAD_VIEW_LOGS,
            CommandId::JOB_SUSPEND,
            CommandId::JOB_RESUME,
        ],
        false,
        |_| json!({"metadata": {}, "spec": {"suspend": true, "template": template("seed-job", "Never")}}),
    ),
    workload(
        "batch",
        "CronJob",
        "cronjobs",
        &[
            "Name",
            "Namespace",
            "Schedule",
            "Timezone",
            "Suspend",
            "Active",
            "Last Schedule",
            "Age",
            "Labels*",
        ],
        &[
            CommandId::CRONJOB_TRIGGER,
            CommandId::CRONJOB_SUSPEND,
            CommandId::CRONJOB_RESUME,
        ],
        false,
        |_| {
            json!({"metadata": {}, "spec": {
                // Only on 1 January, and suspended: the controller never runs it on its own.
                "schedule": "0 0 1 1 *",
                "suspend": true,
                "jobTemplate": {"spec": {"template": template("seed-cronjob", "Never")}},
            }})
        },
    ),
];

/// The pod template every workload seed runs: the pause image, labelled `app: <app>`. Each seed
/// has its own `app`, so no controller adopts (and deletes) another seed's pod.
fn template(app: &str, restart: &str) -> Value {
    json!({
        "metadata": {"labels": {"app": app}},
        "spec": {
            "restartPolicy": restart,
            "terminationGracePeriodSeconds": 0,
            "containers": [{"name": "main", "image": PAUSE}],
        },
    })
}

/// A Deployment, StatefulSet or ReplicaSet selecting `app`, with `spec` merged over the
/// selector and template.
fn controller(app: &str, spec: Value) -> Value {
    let mut full = json!({
        "selector": {"matchLabels": {"app": app}},
        "template": template(app, "Always"),
    });
    if let (Some(full), Some(extra)) = (full.as_object_mut(), spec.as_object()) {
        full.extend(extra.clone());
    }
    json!({"metadata": {}, "spec": full})
}

fn pod(_: &Seed<'_>) -> Value {
    // A pod template is a pod's metadata and spec.
    template("seed-pod", "Never")
}

fn daemonset(_: &Seed<'_>) -> Value {
    let mut template = template("seed-daemonset", "Always");
    // A label no node has: the DaemonSet schedules nothing.
    template["spec"]["nodeSelector"] = json!({"oxikube.test/none": "true"});
    json!({"metadata": {}, "spec": {
        "selector": {"matchLabels": {"app": "seed-daemonset"}},
        "template": template,
    }})
}

fn replication_controller(_: &Seed<'_>) -> Value {
    json!({"metadata": {}, "spec": {
        "replicas": 0,
        "selector": {"app": "seed-rc"},
        "template": template("seed-rc", "Always"),
    }})
}
