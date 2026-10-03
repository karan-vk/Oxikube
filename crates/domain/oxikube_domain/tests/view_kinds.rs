//! Table-driven tests for `NodeSummary`, `WorkloadSummary`, `JobSummary` and `CronJobSummary`,
//! plus a property test that no JSON shape under `spec`/`status` panics a constructor.

use jiff::Timestamp;
use oxikube_domain::view::{ConditionStatus, JobStatus, WorkloadKind};
use oxikube_domain::{
    ContainerSummary, CronJobSummary, JobSummary, NodeSummary, PodSummary, Quantity, Resource,
    ViewError, WorkloadSummary,
};
use proptest::prelude::*;
use serde_json::{Value, json};

fn ts(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn res(api_version: &str, kind: &str, metadata: Value, spec: Value, status: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": api_version, "kind": kind, "metadata": metadata, "spec": spec, "status": status
    }))
    .unwrap()
}

// --- nodes -------------------------------------------------------------------------------

fn node(labels: Value, spec: Value, status: Value) -> NodeSummary {
    let meta = json!({"name": "n1", "labels": labels, "creationTimestamp": "2026-09-01T00:00:00Z"});
    NodeSummary::from_resource(&res("v1", "Node", meta, spec, status)).unwrap()
}

fn ready(status: &str) -> Value {
    json!({"type": "Ready", "status": status, "reason": "KubeletReady", "lastTransitionTime": "2026-09-01T00:05:00Z"})
}

#[test]
fn node_roles_status_and_schedulable() {
    struct Case {
        name: &'static str,
        labels: Value,
        spec: Value,
        conditions: Value,
        roles: &'static str,
        status: &'static str,
        schedulable: bool,
        ready: bool,
    }
    let cases = [
        Case {
            name: "control plane with legacy master label",
            labels: json!({
                "node-role.kubernetes.io/control-plane": "",
                "node-role.kubernetes.io/master": "",
                "kubernetes.io/role": "master"
            }),
            spec: json!({}),
            conditions: json!([ready("True")]),
            roles: "control-plane,master",
            status: "Ready",
            schedulable: true,
            ready: true,
        },
        Case {
            name: "worker",
            labels: json!({"node-role.kubernetes.io/worker": "true", "kubernetes.io/hostname": "n1"}),
            spec: json!({}),
            conditions: json!([ready("True")]),
            roles: "worker",
            status: "Ready",
            schedulable: true,
            ready: true,
        },
        Case {
            name: "no roles; empty role suffix ignored",
            labels: json!({"node-role.kubernetes.io/": "", "kubernetes.io/role": ""}),
            spec: json!({}),
            conditions: json!([ready("True")]),
            roles: "<none>",
            status: "Ready",
            schedulable: true,
            ready: true,
        },
        Case {
            name: "cordoned",
            labels: json!({}),
            spec: json!({"unschedulable": true}),
            conditions: json!([ready("True")]),
            roles: "<none>",
            status: "Ready,SchedulingDisabled",
            schedulable: false,
            ready: true,
        },
        Case {
            name: "not ready",
            labels: json!({}),
            spec: json!({}),
            conditions: json!([ready("False")]),
            roles: "<none>",
            status: "NotReady",
            schedulable: true,
            ready: false,
        },
        Case {
            name: "ready unknown and cordoned",
            labels: json!({}),
            spec: json!({"unschedulable": true}),
            conditions: json!([ready("Unknown")]),
            roles: "<none>",
            status: "NotReady,SchedulingDisabled",
            schedulable: false,
            ready: false,
        },
        Case {
            name: "no Ready condition",
            labels: json!({}),
            spec: json!({}),
            conditions: json!([]),
            roles: "<none>",
            status: "Unknown",
            schedulable: true,
            ready: false,
        },
    ];
    for case in cases {
        let n = node(
            case.labels,
            case.spec,
            json!({"conditions": case.conditions}),
        );
        assert_eq!(n.roles_display(), case.roles, "roles: {}", case.name);
        assert_eq!(&*n.status, case.status, "status: {}", case.name);
        assert_eq!(
            n.schedulable, case.schedulable,
            "schedulable: {}",
            case.name
        );
        assert_eq!(n.is_ready(), case.ready, "ready: {}", case.name);
    }
}

#[test]
fn node_pressure_conditions_versions_and_addresses() {
    let n = node(
        json!({}),
        json!({}),
        json!({
            "conditions": [
                {"type": "MemoryPressure", "status": "True", "reason": "KubeletHasInsufficientMemory"},
                {"type": "DiskPressure", "status": "False"},
                {"type": "PIDPressure", "status": "True"},
                {"type": "NetworkUnavailable", "status": "False"},
                ready("True")
            ],
            "nodeInfo": {
                "kubeletVersion": "v1.31.2",
                "osImage": "Ubuntu 24.04 LTS",
                "kernelVersion": "6.8.0-45-generic",
                "containerRuntimeVersion": "containerd://1.7.22",
                "architecture": "arm64",
                "operatingSystem": "linux"
            },
            "addresses": [
                {"type": "Hostname", "address": "n1"},
                {"type": "InternalIP", "address": "172.18.0.2"},
                {"type": "InternalIP", "address": "fc00::2"}
            ],
            "allocatable": {"cpu": "3800m", "memory": "7Gi", "pods": "110", "bogus": "x"}
        }),
    );
    let problems: Vec<&str> = n.problems().map(|c| &*c.kind).collect();
    assert_eq!(problems, ["MemoryPressure", "PIDPressure"]);
    assert_eq!(n.conditions.len(), 5);
    assert_eq!(n.conditions[0].status, ConditionStatus::True);
    assert_eq!(
        n.conditions[0].reason.as_deref(),
        Some("KubeletHasInsufficientMemory")
    );
    assert_eq!(
        n.conditions[4].last_transition,
        Some(ts("2026-09-01T00:05:00Z"))
    );
    assert_eq!(&*n.status, "Ready");
    assert_eq!(n.kubelet_version.as_deref(), Some("v1.31.2"));
    assert_eq!(n.os_image.as_deref(), Some("Ubuntu 24.04 LTS"));
    assert_eq!(n.kernel_version.as_deref(), Some("6.8.0-45-generic"));
    assert_eq!(
        n.container_runtime_version.as_deref(),
        Some("containerd://1.7.22")
    );
    assert_eq!(n.architecture.as_deref(), Some("arm64"));
    assert_eq!(n.operating_system.as_deref(), Some("linux"));
    assert_eq!(n.internal_ip.as_deref(), Some("172.18.0.2"));
    assert_eq!(n.external_ip, None);
    assert_eq!(n.allocatable_cpu, Some(Quantity::parse("3800m").unwrap()));
    assert_eq!(n.allocatable_memory, Some(Quantity::parse("7Gi").unwrap()));
    assert_eq!(n.allocatable_pods, Some(Quantity::parse("110").unwrap()));
    assert_eq!(
        n.age(ts("2026-09-03T00:00:00Z"))
            .unwrap()
            .to_kubectl_string(),
        "2d"
    );
}

// --- workloads -----------------------------------------------------------------------------

fn workload(kind: &str, spec: Value, status: Value) -> WorkloadSummary {
    let meta = json!({"name": "w", "namespace": "apps"});
    WorkloadSummary::from_resource(&res("apps/v1", kind, meta, spec, status)).unwrap()
}

#[test]
fn workload_counts_per_kind() {
    struct Case {
        name: &'static str,
        w: WorkloadSummary,
        kind: WorkloadKind,
        /// desired, current, ready, updated, available
        counts: (u32, u32, u32, u32, u32),
        unavailable: u32,
        settled: bool,
    }
    let cases = [
        Case {
            name: "deployment mid-rollout",
            w: workload(
                "Deployment",
                json!({"replicas": 3}),
                json!({"replicas": 4, "readyReplicas": 3, "updatedReplicas": 1, "availableReplicas": 3, "unavailableReplicas": 1}),
            ),
            kind: WorkloadKind::Deployment,
            counts: (3, 4, 3, 1, 3),
            unavailable: 0,
            settled: false,
        },
        Case {
            name: "deployment rolled out",
            w: workload(
                "Deployment",
                json!({"replicas": 2}),
                json!({"replicas": 2, "readyReplicas": 2, "updatedReplicas": 2, "availableReplicas": 2}),
            ),
            kind: WorkloadKind::Deployment,
            counts: (2, 2, 2, 2, 2),
            unavailable: 0,
            settled: true,
        },
        Case {
            name: "deployment without spec.replicas defaults to 1",
            w: workload("Deployment", json!({}), json!({})),
            kind: WorkloadKind::Deployment,
            counts: (1, 0, 0, 0, 0),
            unavailable: 1,
            settled: false,
        },
        Case {
            name: "statefulset with a partition",
            w: workload(
                "StatefulSet",
                json!({"replicas": 5, "updateStrategy": {"type": "RollingUpdate", "rollingUpdate": {"partition": 3}}}),
                json!({"replicas": 5, "readyReplicas": 5, "currentReplicas": 3, "updatedReplicas": 2, "availableReplicas": 5}),
            ),
            kind: WorkloadKind::StatefulSet,
            counts: (5, 5, 5, 2, 5),
            unavailable: 0,
            settled: false,
        },
        Case {
            name: "daemonset with unavailable pods",
            w: workload(
                "DaemonSet",
                json!({}),
                json!({
                    "desiredNumberScheduled": 6, "currentNumberScheduled": 6, "numberReady": 4,
                    "updatedNumberScheduled": 6, "numberAvailable": 4, "numberUnavailable": 2
                }),
            ),
            kind: WorkloadKind::DaemonSet,
            counts: (6, 6, 4, 6, 4),
            unavailable: 2,
            settled: false,
        },
        Case {
            name: "replicaset scaled to zero",
            w: workload("ReplicaSet", json!({"replicas": 0}), json!({"replicas": 0})),
            kind: WorkloadKind::ReplicaSet,
            counts: (0, 0, 0, 0, 0),
            unavailable: 0,
            settled: true,
        },
        Case {
            name: "replicaset counts current pods as updated",
            w: workload(
                "ReplicaSet",
                json!({"replicas": 3}),
                json!({"replicas": 3, "readyReplicas": 2, "availableReplicas": 2}),
            ),
            kind: WorkloadKind::ReplicaSet,
            counts: (3, 3, 2, 3, 2),
            unavailable: 1,
            settled: false,
        },
    ];
    for c in cases {
        let w = &c.w;
        assert_eq!(w.kind, c.kind, "kind: {}", c.name);
        assert_eq!(
            (w.desired, w.current, w.ready, w.updated, w.available),
            c.counts,
            "counts: {}",
            c.name
        );
        assert_eq!(w.unavailable(), c.unavailable, "unavailable: {}", c.name);
        assert_eq!(w.is_settled(), c.settled, "settled: {}", c.name);
    }
}

#[test]
fn workload_extras() {
    let sts = workload(
        "StatefulSet",
        json!({"replicas": 5, "updateStrategy": {"rollingUpdate": {"partition": 3}}}),
        json!({"readyReplicas": 4}),
    );
    assert_eq!(sts.partition, Some(3));
    assert_eq!(sts.ready_display(), "4/5");
    assert!(!sts.paused);
    assert_eq!(sts.namespace.as_deref(), Some("apps"));

    let paused = workload(
        "Deployment",
        json!({"replicas": 1, "paused": true}),
        json!({}),
    );
    assert!(paused.paused);
    assert_eq!(paused.partition, None);

    let ds = workload(
        "DaemonSet",
        json!({"paused": true, "replicas": 9}),
        json!({}),
    );
    assert!(!ds.paused, "paused only applies to deployments");
    assert_eq!(ds.desired, 0, "daemonsets ignore spec.replicas");
}

// --- jobs ----------------------------------------------------------------------------------

fn job(spec: Value, status: Value, deleting: bool) -> JobSummary {
    let mut meta =
        json!({"name": "j", "namespace": "batch", "creationTimestamp": "2026-10-01T00:00:00Z"});
    if deleting {
        meta["deletionTimestamp"] = json!("2026-10-01T01:00:00Z");
    }
    JobSummary::from_resource(&res("batch/v1", "Job", meta, spec, status)).unwrap()
}

fn jc(kind: &str, status: &str) -> Value {
    json!({"type": kind, "status": status})
}

#[test]
fn job_status_and_completions() {
    struct Case {
        name: &'static str,
        j: JobSummary,
        status: JobStatus,
        completions: &'static str,
    }
    let cases = [
        Case {
            name: "running",
            j: job(
                json!({"completions": 3, "parallelism": 2}),
                json!({"active": 2, "succeeded": 1, "startTime": "2026-10-01T00:00:10Z"}),
                false,
            ),
            status: JobStatus::Running,
            completions: "1/3",
        },
        Case {
            name: "complete",
            j: job(
                json!({"completions": 1}),
                json!({"succeeded": 1, "conditions": [jc("SuccessCriteriaMet", "True"), jc("Complete", "True")]}),
                false,
            ),
            status: JobStatus::Complete,
            completions: "1/1",
        },
        Case {
            name: "failed",
            j: job(
                json!({"backoffLimit": 2}),
                json!({"failed": 3, "conditions": [jc("FailureTarget", "True"), jc("Failed", "True")]}),
                false,
            ),
            status: JobStatus::Failed,
            completions: "0/1",
        },
        Case {
            name: "failure target before the pods are gone",
            j: job(
                json!({}),
                json!({"failed": 3, "conditions": [jc("FailureTarget", "True")]}),
                false,
            ),
            status: JobStatus::FailureTarget,
            completions: "0/1",
        },
        Case {
            name: "success criteria met",
            j: job(
                json!({}),
                json!({"conditions": [jc("SuccessCriteriaMet", "True")]}),
                false,
            ),
            status: JobStatus::SuccessCriteriaMet,
            completions: "0/1",
        },
        Case {
            name: "suspended",
            j: job(
                json!({"suspend": true, "parallelism": 4}),
                json!({"conditions": [jc("Suspended", "True")]}),
                false,
            ),
            status: JobStatus::Suspended,
            completions: "0/1 of 4",
        },
        Case {
            name: "suspended condition false is running",
            j: job(
                json!({}),
                json!({"conditions": [jc("Suspended", "False")]}),
                false,
            ),
            status: JobStatus::Running,
            completions: "0/1",
        },
        Case {
            name: "terminating",
            j: job(json!({}), json!({"active": 1}), true),
            status: JobStatus::Terminating,
            completions: "0/1",
        },
        Case {
            name: "complete wins over terminating",
            j: job(
                json!({}),
                json!({"conditions": [jc("Complete", "True")]}),
                true,
            ),
            status: JobStatus::Complete,
            completions: "0/1",
        },
    ];
    for c in cases {
        assert_eq!(c.j.status, c.status, "status: {}", c.name);
        assert_eq!(
            c.j.completions_display(),
            c.completions,
            "completions: {}",
            c.name
        );
    }
}

#[test]
fn job_counts_and_duration() {
    let now = ts("2026-10-01T00:30:00Z");
    let active = job(
        json!({"completions": 5, "parallelism": 2, "suspend": false}),
        json!({"active": 2, "succeeded": 1, "failed": 1, "startTime": "2026-10-01T00:00:00Z"}),
        false,
    );
    assert_eq!((active.active, active.succeeded, active.failed), (2, 1, 1));
    assert_eq!((active.completions, active.parallelism), (Some(5), Some(2)));
    assert!(!active.suspend);
    assert_eq!(active.duration(now).unwrap().to_kubectl_string(), "30m");
    assert_eq!(active.status.to_string(), "Running");

    let done = job(
        json!({}),
        json!({
            "succeeded": 1, "startTime": "2026-10-01T00:00:00Z",
            "completionTime": "2026-10-01T00:00:42Z", "conditions": [jc("Complete", "True")]
        }),
        false,
    );
    assert_eq!(done.duration(now).unwrap().to_kubectl_string(), "42s");

    let unstarted = job(json!({"suspend": true}), json!({}), false);
    assert!(unstarted.suspend);
    assert_eq!(unstarted.duration(now), None);
    assert_eq!(unstarted.age(now).unwrap().to_kubectl_string(), "30m");
}

#[test]
fn cronjob_fields() {
    let meta = json!({"name": "nightly", "namespace": "batch"});
    let cj = CronJobSummary::from_resource(&res(
        "batch/v1",
        "CronJob",
        meta.clone(),
        json!({"schedule": "0 3 * * *", "timeZone": "Asia/Tokyo", "suspend": true}),
        json!({
            "active": [{"kind": "Job", "name": "nightly-1"}, {"kind": "Job", "name": "nightly-2"}],
            "lastScheduleTime": "2026-10-01T03:00:00Z",
            "lastSuccessfulTime": "2026-09-30T03:00:12Z"
        }),
    ))
    .unwrap();
    assert_eq!(&*cj.schedule, "0 3 * * *");
    assert_eq!(cj.time_zone.as_deref(), Some("Asia/Tokyo"));
    assert!(cj.suspend);
    assert_eq!(cj.active, 2);
    assert_eq!(cj.last_successful, Some(ts("2026-09-30T03:00:12Z")));
    assert_eq!(
        cj.since_last_schedule(ts("2026-10-01T08:00:00Z"))
            .unwrap()
            .to_kubectl_string(),
        "5h"
    );

    let fresh = CronJobSummary::from_resource(&res(
        "batch/v1",
        "CronJob",
        meta,
        json!({"schedule": "@hourly"}),
        json!({}),
    ))
    .unwrap();
    assert!(!fresh.suspend);
    assert_eq!(fresh.active, 0);
    assert_eq!(fresh.time_zone, None);
    assert_eq!(fresh.since_last_schedule(ts("2026-10-01T05:00:00Z")), None);
}

// --- kind checks and malformed input -------------------------------------------------------

#[test]
fn constructors_reject_other_kinds() {
    let cm = res(
        "v1",
        "ConfigMap",
        json!({"name": "c"}),
        json!({}),
        json!({}),
    );
    assert_eq!(
        NodeSummary::from_resource(&cm),
        Err(ViewError::WrongKind {
            expected: "Node",
            found: "ConfigMap".into()
        })
    );
    assert!(WorkloadSummary::from_resource(&cm).is_err());
    assert!(JobSummary::from_resource(&cm).is_err());
    assert!(CronJobSummary::from_resource(&cm).is_err());

    // Same kind name, different group.
    let foreign = res(
        "example.com/v1",
        "Deployment",
        json!({"name": "d"}),
        json!({}),
        json!({}),
    );
    let err = WorkloadSummary::from_resource(&foreign).unwrap_err();
    assert_eq!(
        err.to_string(),
        "expected Deployment, StatefulSet, DaemonSet or ReplicaSet, got example.com/Deployment"
    );
}

#[test]
fn malformed_inputs_degrade_to_defaults() {
    let meta = json!({"name": "x"});
    let shapes = [
        (json!(null), json!(null)),
        (json!("oops"), json!(42)),
        (
            json!([1, 2]),
            json!({"conditions": {}, "active": "two", "addresses": 3}),
        ),
        (
            json!({"replicas": -2, "completions": "3", "unschedulable": "yes", "updateStrategy": 1}),
            json!({"readyReplicas": 1.5, "conditions": [1, null, {"type": 3}], "nodeInfo": []}),
        ),
    ];
    for (spec, status) in shapes {
        let n = NodeSummary::from_resource(&res(
            "v1",
            "Node",
            meta.clone(),
            spec.clone(),
            status.clone(),
        ))
        .unwrap();
        assert_eq!(&*n.status, "Unknown");
        assert!(n.schedulable);
        for kind in ["Deployment", "StatefulSet", "DaemonSet", "ReplicaSet"] {
            let w = WorkloadSummary::from_resource(&res(
                "apps/v1",
                kind,
                meta.clone(),
                spec.clone(),
                status.clone(),
            ))
            .unwrap();
            assert_eq!(w.ready, 0);
        }
        let j = JobSummary::from_resource(&res(
            "batch/v1",
            "Job",
            meta.clone(),
            spec.clone(),
            status.clone(),
        ))
        .unwrap();
        assert_eq!(j.status, JobStatus::Running);
        let cj =
            CronJobSummary::from_resource(&res("batch/v1", "CronJob", meta.clone(), spec, status))
                .unwrap();
        assert_eq!(cj.active, 0);
    }
}

fn arb_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        (-1e6f64..1e6).prop_map(Value::from),
        prop_oneof![
            Just("Running"),
            Just("Completed"),
            Just("True"),
            Just("Ready"),
            Just("Always"),
            Just("PodInitializing"),
            Just("c0"),
            Just("")
        ]
        .prop_map(Value::from),
    ];
    let keys = prop_oneof![
        Just("name"),
        Just("state"),
        Just("waiting"),
        Just("running"),
        Just("terminated"),
        Just("reason"),
        Just("exitCode"),
        Just("signal"),
        Just("restartCount"),
        Just("ready"),
        Just("started"),
        Just("type"),
        Just("status"),
        Just("restartPolicy"),
        Just("lastState"),
        Just("finishedAt"),
    ];
    leaf.prop_recursive(4, 48, 6, move |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::vec((keys.clone(), inner), 0..5).prop_map(|kv| {
                Value::Object(kv.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
            }),
        ]
    })
}

fn arb_section() -> impl Strategy<Value = Value> {
    let fields = prop_oneof![
        Just("phase"),
        Just("reason"),
        Just("conditions"),
        Just("containers"),
        Just("initContainers"),
        Just("containerStatuses"),
        Just("initContainerStatuses"),
        Just("replicas"),
        Just("unschedulable"),
        Just("nodeInfo"),
        Just("active"),
        Just("startTime"),
    ];
    prop_oneof![
        arb_json(),
        prop::collection::vec((fields, arb_json()), 0..6).prop_map(|kv| {
            Value::Object(kv.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
        }),
    ]
}

proptest! {
    #[test]
    fn no_json_shape_panics(spec in arb_section(), status in arb_section(), deleting in any::<bool>()) {
        let mut meta = json!({"name": "x"});
        if deleting {
            meta["deletionTimestamp"] = json!("2026-10-01T00:00:00Z");
        }
        let pod = res("v1", "Pod", meta.clone(), spec.clone(), status.clone());
        let s = PodSummary::from_resource(&pod).unwrap();
        prop_assert!(!s.status.is_empty());
        let _ = ContainerSummary::list_from_resource(&pod).unwrap();
        let _ = NodeSummary::from_resource(&res("v1", "Node", meta.clone(), spec.clone(), status.clone())).unwrap();
        let _ = WorkloadSummary::from_resource(&res("apps/v1", "Deployment", meta.clone(), spec.clone(), status.clone())).unwrap();
        let _ = JobSummary::from_resource(&res("batch/v1", "Job", meta.clone(), spec.clone(), status.clone())).unwrap();
        let _ = CronJobSummary::from_resource(&res("batch/v1", "CronJob", meta, spec, status)).unwrap();
    }
}
