//! Pods and the controllers that run them.
//!
//! Sources: Freelens Pod / Deployment / DaemonSet / StatefulSet / ReplicaSet / ReplicationController
//! / Job / CronJob lists (A1); k9s `po`, `dp`, `rs`, `sts`/`ds`, `job`/`cj` (A2); kubectl printers.

use super::super::def::{
    AGE, CPU, ColumnDef, KindDef, LABELS, MEMORY, NAME, NAMESPACE,
    Src::{Func, Text},
};
use super::super::funcs as f;

const POD: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("ready", "Ready", Func(f::pod_ready)).number(),
    ColumnDef::new("status", "Status", Func(f::pod_status)),
    ColumnDef::new("restarts", "Restarts", Func(f::pod_restarts)).number(),
    CPU,
    MEMORY,
    ColumnDef::new("node", "Node", Text("/spec/nodeName")),
    ColumnDef::new("ip", "IP", Func(f::pod_ip)),
    AGE,
    ColumnDef::new("qos", "QoS", Text("/status/qosClass")).wide(),
    ColumnDef::new("controlled-by", "Controlled By", Func(f::controlled_by)).wide(),
    ColumnDef::new(
        "nominated-node",
        "Nominated Node",
        Text("/status/nominatedNodeName"),
    )
    .wide(),
    ColumnDef::new(
        "service-account",
        "Service Account",
        Text("/spec/serviceAccountName"),
    )
    .wide(),
    ColumnDef::new("last-restart", "Last Restart", Func(f::pod_last_restart))
        .age()
        .wide(),
    LABELS,
];

const DEPLOYMENT: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("ready", "Ready", Func(f::workload_ready)).number(),
    ColumnDef::new("up-to-date", "Up-to-date", Func(f::workload_updated)).number(),
    ColumnDef::new("available", "Available", Func(f::workload_available)).number(),
    AGE,
    ColumnDef::new("images", "Images", Func(f::workload_images)).wide(),
    ColumnDef::new("selector", "Selector", Func(f::workload_selector)).wide(),
    LABELS,
];

const REPLICASET: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("desired", "Desired", Func(f::workload_desired)).number(),
    ColumnDef::new("current", "Current", Func(f::workload_current)).number(),
    ColumnDef::new("ready", "Ready", Func(f::workload_ready_count)).number(),
    AGE,
    ColumnDef::new("images", "Images", Func(f::workload_images)).wide(),
    ColumnDef::new("selector", "Selector", Func(f::workload_selector)).wide(),
    ColumnDef::new("controlled-by", "Controlled By", Func(f::controlled_by)).wide(),
    LABELS,
];

const STATEFULSET: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("ready", "Ready", Func(f::workload_ready)).number(),
    AGE,
    ColumnDef::new("images", "Images", Func(f::workload_images)).wide(),
    LABELS,
];

const DAEMONSET: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("desired", "Desired", Func(f::workload_desired)).number(),
    ColumnDef::new("current", "Current", Func(f::workload_current)).number(),
    ColumnDef::new("ready", "Ready", Func(f::workload_ready_count)).number(),
    ColumnDef::new("up-to-date", "Up-to-date", Func(f::workload_updated)).number(),
    ColumnDef::new("available", "Available", Func(f::workload_available)).number(),
    ColumnDef::new("node-selector", "Node Selector", Func(f::node_selector)),
    AGE,
    ColumnDef::new("images", "Images", Func(f::workload_images)).wide(),
    LABELS,
];

const REPLICATION_CONTROLLER: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("desired", "Desired", Func(f::rc_desired)).number(),
    ColumnDef::new("current", "Current", Func(f::rc_current)).number(),
    ColumnDef::new("ready", "Ready", Func(f::rc_ready)).number(),
    AGE,
    ColumnDef::new("selector", "Selector", Func(f::rc_selector)).wide(),
    LABELS,
];

const JOB: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("status", "Status", Func(f::job_status)),
    ColumnDef::new("completions", "Completions", Func(f::job_completions)).number(),
    ColumnDef::new("duration", "Duration", Func(f::job_duration)).age(),
    AGE,
    LABELS,
];

const CRONJOB: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("schedule", "Schedule", Text("/spec/schedule")),
    ColumnDef::new("suspend", "Suspend", Func(f::cron_suspend)),
    ColumnDef::new("active", "Active", Func(f::cron_active)).number(),
    ColumnDef::new(
        "last-schedule",
        "Last Schedule",
        Func(f::cron_last_schedule),
    )
    .age(),
    AGE,
    ColumnDef::new("timezone", "Timezone", Text("/spec/timeZone")).wide(),
    LABELS,
];

pub(super) const KINDS: &[KindDef] = &[
    KindDef {
        group: "",
        kind: "Pod",
        columns: POD,
    },
    KindDef {
        group: "apps",
        kind: "Deployment",
        columns: DEPLOYMENT,
    },
    KindDef {
        group: "apps",
        kind: "ReplicaSet",
        columns: REPLICASET,
    },
    KindDef {
        group: "apps",
        kind: "StatefulSet",
        columns: STATEFULSET,
    },
    KindDef {
        group: "apps",
        kind: "DaemonSet",
        columns: DAEMONSET,
    },
    KindDef {
        group: "",
        kind: "ReplicationController",
        columns: REPLICATION_CONTROLLER,
    },
    KindDef {
        group: "batch",
        kind: "Job",
        columns: JOB,
    },
    KindDef {
        group: "batch",
        kind: "CronJob",
        columns: CRONJOB,
    },
];
