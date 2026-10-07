//! Pod health honours container state: the same inputs `PodSummary` reads for `READY` and
//! `STATUS`, so a pod the table shows as `CrashLoopBackOff` is never counted healthy.

use oxikube_domain::Resource;
use oxikube_domain::view::{Health, PodSummary, health_of};
use serde_json::{Value, json};

const NOW: &str = "2026-01-01T00:00:00Z";

fn pod(extra_meta: Value, spec: Value, status: Value) -> Resource {
    let mut meta = json!({"name": "p", "namespace": "d", "creationTimestamp": NOW});
    meta.as_object_mut()
        .unwrap()
        .extend(extra_meta.as_object().unwrap().clone());
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod", "metadata": meta, "spec": spec, "status": status,
    }))
    .expect("a pod")
}

fn two_containers() -> Value {
    json!({"containers": [{"name": "app"}, {"name": "side"}]})
}

fn cs(name: &str, ready: bool, state: Value) -> Value {
    json!({"name": name, "ready": ready, "restartCount": 0, "state": state})
}

fn running() -> Value {
    json!({"running": {"startedAt": NOW}})
}

fn waiting(reason: &str) -> Value {
    json!({"waiting": {"reason": reason}})
}

fn terminated(reason: &str, exit_code: i32) -> Value {
    json!({"terminated": {"reason": reason, "exitCode": exit_code}})
}

fn status(phase: &str, containers: Vec<Value>) -> Value {
    json!({"phase": phase, "containerStatuses": containers})
}

struct Case {
    name: &'static str,
    pod: Resource,
    /// The `STATUS` column the table shows for the same pod.
    table_status: &'static str,
    healthy: bool,
}

fn cases() -> Vec<Case> {
    let one = json!({"containers": [{"name": "app"}]});
    let deleting = json!({"deletionTimestamp": NOW});
    vec![
        Case {
            name: "running and ready",
            pod: pod(
                json!({}),
                two_containers(),
                status(
                    "Running",
                    vec![cs("app", true, running()), cs("side", true, running())],
                ),
            ),
            table_status: "Running",
            healthy: true,
        },
        Case {
            name: "CrashLoopBackOff",
            pod: pod(
                json!({}),
                one.clone(),
                status(
                    "Running",
                    vec![cs("app", false, waiting("CrashLoopBackOff"))],
                ),
            ),
            table_status: "CrashLoopBackOff",
            healthy: false,
        },
        Case {
            name: "OOMKilled",
            pod: pod(
                json!({}),
                one.clone(),
                status(
                    "Running",
                    vec![cs("app", false, terminated("OOMKilled", 137))],
                ),
            ),
            table_status: "OOMKilled",
            healthy: false,
        },
        Case {
            name: "ImagePullBackOff",
            pod: pod(
                json!({}),
                one.clone(),
                status(
                    "Pending",
                    vec![cs("app", false, waiting("ImagePullBackOff"))],
                ),
            ),
            table_status: "ImagePullBackOff",
            healthy: false,
        },
        Case {
            name: "ImagePullBackOff while the phase still says Running",
            pod: pod(
                json!({}),
                two_containers(),
                status(
                    "Running",
                    vec![
                        cs("app", true, running()),
                        cs("side", false, waiting("ImagePullBackOff")),
                    ],
                ),
            ),
            table_status: "ImagePullBackOff",
            healthy: false,
        },
        Case {
            name: "Terminating",
            pod: pod(
                deleting,
                one.clone(),
                status("Running", vec![cs("app", true, running())]),
            ),
            table_status: "Terminating",
            healthy: false,
        },
        Case {
            name: "running but not ready",
            pod: pod(
                json!({}),
                two_containers(),
                status(
                    "Running",
                    vec![cs("app", true, running()), cs("side", false, running())],
                ),
            ),
            table_status: "Running",
            healthy: false,
        },
        Case {
            name: "container with no status yet",
            pod: pod(
                json!({}),
                two_containers(),
                status("Running", vec![cs("app", true, running())]),
            ),
            table_status: "Running",
            healthy: false,
        },
        Case {
            name: "Completed container beside a ready one",
            pod: pod(json!({}), two_containers(), {
                let mut st = status(
                    "Running",
                    vec![
                        cs("app", true, running()),
                        cs("side", false, terminated("Completed", 0)),
                    ],
                );
                st["conditions"] = json!([{"type": "Ready", "status": "True"}]);
                st
            }),
            table_status: "Running",
            healthy: true,
        },
        Case {
            name: "Completed (phase Succeeded)",
            pod: pod(
                json!({}),
                one.clone(),
                status(
                    "Succeeded",
                    vec![cs("app", false, terminated("Completed", 0))],
                ),
            ),
            table_status: "Completed",
            healthy: true,
        },
        Case {
            name: "Succeeded with no container statuses",
            pod: pod(json!({}), json!({}), json!({"phase": "Succeeded"})),
            table_status: "Succeeded",
            healthy: true,
        },
        Case {
            name: "Failed",
            pod: pod(
                json!({}),
                one.clone(),
                status("Failed", vec![cs("app", false, terminated("Error", 1))]),
            ),
            table_status: "Error",
            healthy: false,
        },
        Case {
            name: "ContainerStatusUnknown",
            pod: pod(
                json!({}),
                one,
                status(
                    "Running",
                    vec![cs("app", false, terminated("ContainerStatusUnknown", 137))],
                ),
            ),
            table_status: "ContainerStatusUnknown",
            healthy: false,
        },
    ]
}

#[test]
fn pod_health_follows_container_state() {
    for case in cases() {
        let verdict = health_of(&case.pod).expect("a pod has a rule");
        assert_eq!(verdict.is_healthy(), case.healthy, "{}", case.name);
        let table = PodSummary::from_resource(&case.pod).expect("a pod summary");
        assert_eq!(
            &*table.status, case.table_status,
            "{}: table STATUS",
            case.name
        );
    }
}

#[test]
fn a_ready_sidecar_is_required_but_a_plain_init_container_is_not() {
    let spec = json!({
        "initContainers": [
            {"name": "setup"},
            {"name": "proxy", "restartPolicy": "Always"},
        ],
        "containers": [{"name": "app"}],
    });
    let build = |proxy_ready: bool| {
        pod(
            json!({}),
            spec.clone(),
            json!({
                "phase": "Running",
                "initContainerStatuses": [
                    cs("setup", false, terminated("Completed", 0)),
                    cs("proxy", proxy_ready, running()),
                ],
                "containerStatuses": [cs("app", true, running())],
            }),
        )
    };
    assert_eq!(health_of(&build(true)), Some(Health::Healthy));
    assert_eq!(health_of(&build(false)), Some(Health::Unhealthy));
}
