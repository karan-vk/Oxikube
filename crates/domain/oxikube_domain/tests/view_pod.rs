//! Table-driven tests for `PodSummary` (kubectl pod printer rules) and `ContainerSummary`.

use jiff::Timestamp;
use oxikube_domain::view::{ContainerKind, ContainerState, PodPhase, QosClass};
use oxikube_domain::{ContainerSummary, PodSummary, Resource, ViewError};
use serde_json::{Value, json};

const DELETING: &str = "2026-10-03T12:00:00Z";

fn ts(s: &str) -> Timestamp {
    s.parse().unwrap()
}

/// A Pod with the given `spec` and `status`; `deletion` sets `deletionTimestamp`.
fn pod_json(spec: Value, status: Value, deletion: Option<&str>) -> Value {
    let mut meta = json!({
        "name": "p",
        "namespace": "default",
        "creationTimestamp": "2026-10-01T00:00:00Z"
    });
    if let Some(d) = deletion {
        meta["deletionTimestamp"] = json!(d);
    }
    json!({"apiVersion": "v1", "kind": "Pod", "metadata": meta, "spec": spec, "status": status})
}

fn summary(json: Value) -> PodSummary {
    PodSummary::from_resource(&Resource::from_json(json).unwrap()).unwrap()
}

/// `spec` with `n` regular containers `c0..cn`.
fn containers(n: usize) -> Value {
    let list: Vec<Value> = (0..n)
        .map(|i| json!({"name": format!("c{i}"), "image": "busybox"}))
        .collect();
    json!({"containers": list})
}

/// `spec` with init containers (`(name, sidecar)`) and `n` regular containers.
fn with_inits(inits: &[(&str, bool)], n: usize) -> Value {
    let mut spec = containers(n);
    let list: Vec<Value> = inits
        .iter()
        .map(|(name, sidecar)| {
            if *sidecar {
                json!({"name": name, "image": "envoy", "restartPolicy": "Always"})
            } else {
                json!({"name": name, "image": "busybox"})
            }
        })
        .collect();
    spec["initContainers"] = json!(list);
    spec
}

fn running(name: &str, ready: bool, restarts: u32) -> Value {
    json!({
        "name": name, "ready": ready, "started": true, "restartCount": restarts,
        "state": {"running": {"startedAt": "2026-10-01T00:01:00Z"}}
    })
}

fn waiting(name: &str, reason: &str, restarts: u32) -> Value {
    json!({
        "name": name, "ready": false, "started": false, "restartCount": restarts,
        "state": {"waiting": {"reason": reason, "message": "m"}}
    })
}

fn terminated(name: &str, exit_code: i32, reason: Option<&str>) -> Value {
    let mut t = json!({"exitCode": exit_code, "finishedAt": "2026-10-01T00:02:00Z"});
    if let Some(r) = reason {
        t["reason"] = json!(r);
    }
    json!({"name": name, "ready": false, "started": false, "restartCount": 0, "state": {"terminated": t}})
}

fn signalled(name: &str, signal: i32) -> Value {
    json!({
        "name": name, "ready": false, "restartCount": 0,
        "state": {"terminated": {"exitCode": 128 + signal, "signal": signal}}
    })
}

fn cond(kind: &str, status: &str) -> Value {
    json!({"type": kind, "status": status})
}

struct Case {
    name: &'static str,
    pod: Value,
    status: &'static str,
    ready: (u32, u32),
    restarts: u32,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "pending, nothing reported",
            pod: pod_json(containers(1), json!({"phase": "Pending"}), None),
            status: "Pending",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "running and ready",
            pod: pod_json(
                containers(2),
                json!({"phase": "Running", "containerStatuses": [running("c0", true, 0), running("c1", true, 1)]}),
                None,
            ),
            status: "Running",
            ready: (2, 2),
            restarts: 1,
        },
        Case {
            name: "running but one container not ready",
            pod: pod_json(
                containers(2),
                json!({"phase": "Running", "containerStatuses": [running("c0", true, 0), running("c1", false, 0)]}),
                None,
            ),
            status: "Running",
            ready: (1, 2),
            restarts: 0,
        },
        Case {
            name: "succeeded with no container statuses",
            pod: pod_json(containers(1), json!({"phase": "Succeeded"}), None),
            status: "Succeeded",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "failed with no container statuses",
            pod: pod_json(containers(1), json!({"phase": "Failed"}), None),
            status: "Failed",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "status.reason overrides phase (evicted)",
            pod: pod_json(
                containers(1),
                json!({"phase": "Failed", "reason": "Evicted"}),
                None,
            ),
            status: "Evicted",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "scheduling gated",
            pod: pod_json(
                containers(1),
                json!({"phase": "Pending", "conditions": [{"type": "PodScheduled", "status": "False", "reason": "SchedulingGated"}]}),
                None,
            ),
            status: "SchedulingGated",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "container creating",
            pod: pod_json(
                containers(1),
                json!({"phase": "Pending", "containerStatuses": [waiting("c0", "ContainerCreating", 0)]}),
                None,
            ),
            status: "ContainerCreating",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "crash loop back-off",
            pod: pod_json(
                containers(1),
                json!({"phase": "Running", "containerStatuses": [waiting("c0", "CrashLoopBackOff", 7)]}),
                None,
            ),
            status: "CrashLoopBackOff",
            ready: (0, 1),
            restarts: 7,
        },
        Case {
            name: "image pull back-off",
            pod: pod_json(
                containers(1),
                json!({"phase": "Pending", "containerStatuses": [waiting("c0", "ImagePullBackOff", 0)]}),
                None,
            ),
            status: "ImagePullBackOff",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "first container's reason wins",
            pod: pod_json(
                containers(2),
                json!({"phase": "Pending", "containerStatuses": [waiting("c0", "ErrImagePull", 0), waiting("c1", "ContainerCreating", 0)]}),
                None,
            ),
            status: "ErrImagePull",
            ready: (0, 2),
            restarts: 0,
        },
        Case {
            name: "oom killed",
            pod: pod_json(
                containers(1),
                json!({"phase": "Running", "containerStatuses": [terminated("c0", 137, Some("OOMKilled"))]}),
                None,
            ),
            status: "OOMKilled",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "completed",
            pod: pod_json(
                containers(1),
                json!({"phase": "Succeeded", "containerStatuses": [terminated("c0", 0, Some("Completed"))]}),
                None,
            ),
            status: "Completed",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "error",
            pod: pod_json(
                containers(1),
                json!({"phase": "Failed", "containerStatuses": [terminated("c0", 1, Some("Error"))]}),
                None,
            ),
            status: "Error",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "terminated without reason shows exit code",
            pod: pod_json(
                containers(1),
                json!({"phase": "Failed", "containerStatuses": [terminated("c0", 3, None)]}),
                None,
            ),
            status: "ExitCode:3",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "terminated by signal without reason",
            pod: pod_json(
                containers(1),
                json!({"phase": "Failed", "containerStatuses": [signalled("c0", 9)]}),
                None,
            ),
            status: "Signal:9",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "completed container next to a running one, pod ready",
            pod: pod_json(
                containers(2),
                json!({
                    "phase": "Running",
                    "conditions": [cond("Ready", "True")],
                    "containerStatuses": [terminated("c0", 0, Some("Completed")), running("c1", true, 0)]
                }),
                None,
            ),
            status: "Running",
            ready: (1, 2),
            restarts: 0,
        },
        Case {
            name: "completed container next to a running one, pod not ready",
            pod: pod_json(
                containers(2),
                json!({
                    "phase": "Running",
                    "conditions": [cond("Ready", "False")],
                    "containerStatuses": [terminated("c0", 0, Some("Completed")), running("c1", true, 0)]
                }),
                None,
            ),
            status: "NotReady",
            ready: (1, 2),
            restarts: 0,
        },
        Case {
            name: "completed container next to a failed one shows the error",
            pod: pod_json(
                containers(2),
                json!({
                    "phase": "Failed",
                    "containerStatuses": [terminated("c0", 0, Some("Completed")), terminated("c1", 2, Some("Error"))]
                }),
                None,
            ),
            status: "Error",
            ready: (0, 2),
            restarts: 0,
        },
        Case {
            name: "terminating",
            pod: pod_json(
                containers(1),
                json!({"phase": "Running", "containerStatuses": [running("c0", true, 0)]}),
                Some(DELETING),
            ),
            status: "Terminating",
            ready: (1, 1),
            restarts: 0,
        },
        Case {
            name: "terminating is not shown for a finished pod",
            pod: pod_json(
                containers(1),
                json!({"phase": "Succeeded", "containerStatuses": [terminated("c0", 0, Some("Completed"))]}),
                Some(DELETING),
            ),
            status: "Completed",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "node lost while deleting shows Unknown",
            pod: pod_json(
                containers(1),
                json!({"phase": "Running", "reason": "NodeLost", "containerStatuses": [running("c0", true, 0)]}),
                Some(DELETING),
            ),
            status: "Unknown",
            ready: (1, 1),
            restarts: 0,
        },
        Case {
            name: "node lost without deletion keeps the reason",
            pod: pod_json(
                containers(1),
                json!({"phase": "Running", "reason": "NodeLost"}),
                None,
            ),
            status: "NodeLost",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "Init:0/2 while the first init container runs",
            pod: pod_json(
                with_inits(&[("i0", false), ("i1", false)], 1),
                json!({
                    "phase": "Pending",
                    "initContainerStatuses": [running("i0", false, 0), waiting("i1", "PodInitializing", 0)],
                    "containerStatuses": [waiting("c0", "PodInitializing", 0)]
                }),
                None,
            ),
            status: "Init:0/2",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "Init:1/2 after the first init container exits 0",
            pod: pod_json(
                with_inits(&[("i0", false), ("i1", false)], 1),
                json!({
                    "phase": "Pending",
                    "initContainerStatuses": [terminated("i0", 0, Some("Completed")), running("i1", false, 0)]
                }),
                None,
            ),
            status: "Init:1/2",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "init container waiting for PodInitializing shows the index",
            pod: pod_json(
                with_inits(&[("i0", false)], 1),
                json!({"phase": "Pending", "initContainerStatuses": [waiting("i0", "PodInitializing", 0)]}),
                None,
            ),
            status: "Init:0/1",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "Init:Error",
            pod: pod_json(
                with_inits(&[("i0", false), ("i1", false)], 1),
                json!({
                    "phase": "Pending",
                    "initContainerStatuses": [terminated("i0", 1, Some("Error")), waiting("i1", "PodInitializing", 0)]
                }),
                None,
            ),
            status: "Init:Error",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "Init:CrashLoopBackOff",
            pod: pod_json(
                with_inits(&[("i0", false)], 1),
                json!({"phase": "Pending", "initContainerStatuses": [waiting("i0", "CrashLoopBackOff", 4)]}),
                None,
            ),
            status: "Init:CrashLoopBackOff",
            ready: (0, 1),
            restarts: 4,
        },
        Case {
            name: "Init:ExitCode:N",
            pod: pod_json(
                with_inits(&[("i0", false)], 1),
                json!({"phase": "Pending", "initContainerStatuses": [terminated("i0", 2, None)]}),
                None,
            ),
            status: "Init:ExitCode:2",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "Init:Signal:N",
            pod: pod_json(
                with_inits(&[("i0", false)], 1),
                json!({"phase": "Pending", "initContainerStatuses": [signalled("i0", 15)]}),
                None,
            ),
            status: "Init:Signal:15",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "PodInitializing after init containers finish",
            pod: pod_json(
                with_inits(&[("i0", false)], 1),
                json!({
                    "phase": "Pending",
                    "conditions": [cond("Initialized", "True")],
                    "initContainerStatuses": [terminated("i0", 0, Some("Completed"))],
                    "containerStatuses": [waiting("c0", "PodInitializing", 0)]
                }),
                None,
            ),
            status: "PodInitializing",
            ready: (0, 1),
            restarts: 0,
        },
        Case {
            name: "restarts count only init containers while initializing",
            pod: pod_json(
                with_inits(&[("i0", false)], 1),
                json!({
                    "phase": "Pending",
                    "initContainerStatuses": [running("i0", false, 3)],
                    "containerStatuses": [json!({"name": "c0", "restartCount": 9, "state": {"waiting": {"reason": "PodInitializing"}}})]
                }),
                None,
            ),
            status: "Init:0/1",
            ready: (0, 1),
            restarts: 3,
        },
        Case {
            name: "Initialized condition counts container restarts despite a pending init",
            pod: pod_json(
                with_inits(&[("i0", false)], 1),
                json!({
                    "phase": "Running",
                    "conditions": [cond("Initialized", "True")],
                    "initContainerStatuses": [running("i0", false, 3)],
                    "containerStatuses": [running("c0", true, 1)]
                }),
                None,
            ),
            status: "Init:0/1",
            ready: (1, 1),
            restarts: 1,
        },
        Case {
            name: "sidecar started, next init container running",
            pod: pod_json(
                with_inits(&[("proxy", true), ("migrate", false)], 1),
                json!({
                    "phase": "Pending",
                    "initContainerStatuses": [running("proxy", true, 0), running("migrate", false, 0)]
                }),
                None,
            ),
            status: "Init:1/2",
            ready: (1, 2),
            restarts: 0,
        },
        Case {
            name: "sidecar not started yet blocks init",
            pod: pod_json(
                with_inits(&[("proxy", true)], 1),
                json!({
                    "phase": "Pending",
                    "initContainerStatuses": [json!({"name": "proxy", "ready": false, "started": false, "restartCount": 0, "state": {"running": {}}})]
                }),
                None,
            ),
            status: "Init:0/1",
            ready: (0, 2),
            restarts: 0,
        },
        Case {
            name: "sidecar and main container running",
            pod: pod_json(
                with_inits(&[("proxy", true), ("migrate", false)], 1),
                json!({
                    "phase": "Running",
                    "conditions": [cond("Initialized", "True"), cond("Ready", "True")],
                    "initContainerStatuses": [running("proxy", true, 2), terminated("migrate", 0, Some("Completed"))],
                    "containerStatuses": [running("c0", true, 1)]
                }),
                None,
            ),
            status: "Running",
            ready: (2, 2),
            restarts: 3,
        },
    ]
}

#[test]
fn pod_status_follows_kubectl_printer() {
    for case in cases() {
        let s = summary(case.pod);
        assert_eq!(&*s.status, case.status, "status: {}", case.name);
        assert_eq!((s.ready, s.total), case.ready, "ready: {}", case.name);
        assert_eq!(s.restarts, case.restarts, "restarts: {}", case.name);
    }
}

/// kdash's `get_status` checks `reason.is_empty()` and then unwraps the reason for a terminated
/// init container, so a non-empty reason (`Error`, `OOMKilled`) fell into the wrong branch.
#[test]
fn regression_init_container_terminated_with_reason() {
    for (reason, exit, expected) in [
        (Some("Error"), 1, "Init:Error"),
        (Some("OOMKilled"), 137, "Init:OOMKilled"),
        (Some("ContainerCannotRun"), 128, "Init:ContainerCannotRun"),
        (None, 1, "Init:ExitCode:1"),
    ] {
        let s = summary(pod_json(
            with_inits(&[("i0", false)], 1),
            json!({"phase": "Pending", "initContainerStatuses": [terminated("i0", exit, reason)]}),
            None,
        ));
        assert_eq!(&*s.status, expected, "reason {reason:?}");
    }
}

#[test]
fn pod_fields_are_read_from_json() {
    let s = summary(pod_json(
        json!({"nodeName": "node-a", "containers": [{"name": "c0", "image": "nginx"}]}),
        json!({
            "phase": "Running",
            "qosClass": "Burstable",
            "podIP": "10.0.0.9",
            "podIPs": [{"ip": "10.0.0.1"}, {"ip": "fd00::1"}],
            "nominatedNodeName": "node-b",
            "containerStatuses": [running("c0", true, 0)]
        }),
        None,
    ));
    assert_eq!(&*s.name, "p");
    assert_eq!(s.namespace.as_deref(), Some("default"));
    assert_eq!(s.phase, PodPhase::Running);
    assert_eq!(s.qos, Some(QosClass::Burstable));
    assert_eq!(s.ip.as_deref(), Some("10.0.0.1"));
    assert_eq!(s.node.as_deref(), Some("node-a"));
    assert_eq!(s.nominated_node.as_deref(), Some("node-b"));
    assert_eq!(s.ready_display(), "1/1");
    assert_eq!(s.created, Some(ts("2026-10-01T00:00:00Z")));
    assert_eq!(
        s.age(ts("2026-10-01T03:00:00Z"))
            .unwrap()
            .to_kubectl_string(),
        "3h"
    );

    let fallback = summary(pod_json(
        containers(1),
        json!({"phase": "Pending", "podIP": "10.0.0.9", "qosClass": "Weird"}),
        None,
    ));
    assert_eq!(fallback.ip.as_deref(), Some("10.0.0.9"));
    assert_eq!(fallback.qos, None);
    assert_eq!(fallback.node, None);
}

#[test]
fn restarts_display_includes_last_restart_age() {
    let mut cs = running("c0", true, 3);
    cs["lastState"] = json!({"terminated": {"exitCode": 1, "finishedAt": "2026-10-01T00:10:00Z"}});
    let s = summary(pod_json(
        containers(1),
        json!({"phase": "Running", "containerStatuses": [cs]}),
        None,
    ));
    assert_eq!(s.last_restart, Some(ts("2026-10-01T00:10:00Z")));
    assert_eq!(s.restarts_display(ts("2026-10-01T00:15:00Z")), "3 (5m ago)");

    let none = summary(pod_json(
        containers(1),
        json!({"phase": "Running", "containerStatuses": [running("c0", true, 2)]}),
        None,
    ));
    assert_eq!(none.restarts_display(ts("2026-10-01T00:15:00Z")), "2");
}

#[test]
fn container_summaries_join_spec_and_status() {
    let mut spec = with_inits(&[("init", false), ("proxy", true)], 2);
    spec["ephemeralContainers"] = json!([{"name": "debug", "image": "busybox"}]);
    spec["containers"][1] = json!({"name": "c1"});
    let mut c0 = waiting("c0", "CrashLoopBackOff", 4);
    c0["lastState"] = json!({"terminated": {"exitCode": 137, "reason": "OOMKilled", "finishedAt": "2026-10-01T00:10:00Z"}});
    let mut c1 = running("c1", true, 0);
    c1["image"] = json!("docker.io/library/redis:7");
    let res = Resource::from_json(pod_json(
        spec,
        json!({
            "phase": "Running",
            "initContainerStatuses": [terminated("init", 0, Some("Completed")), running("proxy", true, 0)],
            "containerStatuses": [c0, c1]
        }),
        None,
    ))
    .unwrap();

    let list = ContainerSummary::list_from_resource(&res).unwrap();
    let names: Vec<&str> = list.iter().map(|c| &*c.name).collect();
    assert_eq!(names, ["init", "proxy", "c0", "c1", "debug"]);
    let kinds: Vec<ContainerKind> = list.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            ContainerKind::Init,
            ContainerKind::Sidecar,
            ContainerKind::Regular,
            ContainerKind::Regular,
            ContainerKind::Ephemeral
        ]
    );

    assert_eq!(list[0].state.label(), "Completed");
    assert!(matches!(
        &list[1].state,
        ContainerState::Running {
            started_at: Some(_)
        }
    ));
    assert!(list[1].ready);

    let c0 = &list[2];
    assert_eq!(c0.state.label(), "CrashLoopBackOff");
    assert_eq!(c0.restarts, 4);
    assert_eq!(c0.image.as_deref(), Some("busybox"));
    let last = c0.last_termination.as_ref().unwrap();
    assert_eq!(
        (last.exit_code, last.reason.as_deref()),
        (137, Some("OOMKilled"))
    );

    assert_eq!(list[3].image.as_deref(), Some("docker.io/library/redis:7"));
    assert!(list[3].started);

    let debug = &list[4];
    assert_eq!(debug.state, ContainerState::Unknown);
    assert_eq!(debug.state.label(), "Unknown");
    assert!(!debug.ready);
}

#[test]
fn wrong_kind_is_rejected() {
    let res = Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "Deployment", "metadata": {"name": "d"}
    }))
    .unwrap();
    assert_eq!(
        PodSummary::from_resource(&res),
        Err(ViewError::WrongKind {
            expected: "Pod",
            found: "apps/Deployment".into()
        })
    );
    assert!(ContainerSummary::list_from_resource(&res).is_err());
}

#[test]
fn malformed_pods_degrade_to_defaults() {
    let bad = [
        json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "p"}}),
        pod_json(json!(5), json!("oops"), None),
        pod_json(
            json!({"containers": {}}),
            json!({"phase": 7, "containerStatuses": {}}),
            None,
        ),
        pod_json(
            json!({"containers": [1, "x", {"name": 3}], "initContainers": [null]}),
            json!({
                "phase": "Running",
                "conditions": "nope",
                "initContainerStatuses": [1, {"name": 2, "state": "x", "restartCount": "3"}],
                "containerStatuses": [null, {"state": {"terminated": "x"}, "restartCount": -4}]
            }),
            Some(DELETING),
        ),
        pod_json(
            containers(1),
            json!({"containerStatuses": [{"name": "c0", "restartCount": 1e30, "state": {"terminated": {"exitCode": 99999999999i64}}}]}),
            None,
        ),
    ];
    for json in bad {
        let res = Resource::from_json(json).unwrap();
        let s = PodSummary::from_resource(&res).unwrap();
        assert!(!s.status.is_empty());
        let _ = ContainerSummary::list_from_resource(&res).unwrap();
    }
    let empty = summary(json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "p"}}));
    assert_eq!(&*empty.status, "Unknown");
    assert_eq!(empty.phase, PodPhase::Unknown);
    assert_eq!((empty.ready, empty.total, empty.restarts), (0, 0, 0));
}

/// Builds 10 000 summaries; the time is printed for the PR (run with `--nocapture`).
#[test]
fn timing_10k_pod_summaries() {
    let template = pod_json(
        with_inits(&[("proxy", true), ("migrate", false)], 2),
        json!({
            "phase": "Running",
            "qosClass": "Burstable",
            "podIP": "10.0.0.1",
            "conditions": [cond("Initialized", "True"), cond("Ready", "True")],
            "initContainerStatuses": [running("proxy", true, 1), terminated("migrate", 0, Some("Completed"))],
            "containerStatuses": [running("c0", true, 0), waiting("c1", "CrashLoopBackOff", 5)]
        }),
        None,
    );
    let pods: Vec<Resource> = (0..10_000)
        .map(|i| {
            let mut j = template.clone();
            j["metadata"]["name"] = json!(format!("pod-{i}"));
            Resource::from_json(j).unwrap()
        })
        .collect();
    let start = std::time::Instant::now();
    let summaries: Vec<PodSummary> = pods
        .iter()
        .map(|p| PodSummary::from_resource(p).unwrap())
        .collect();
    let elapsed = start.elapsed();
    eprintln!("built {} PodSummary values in {elapsed:?}", summaries.len());
    assert_eq!(summaries.len(), 10_000);
    assert!(summaries.iter().all(|s| &*s.status == "CrashLoopBackOff"));
}
