// Test cases derived from Kubernetes (https://github.com/kubernetes/kubernetes),
// `pkg/printers/internalversion/printers_test.go` at `release-1.36`: `TestPrintPod` and
// `TestPrintPodWithRestartableInitContainer`.
//
// Copyright 2014 The Kubernetes Authors.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// Modifications (c) Oxikube contributors: each Go `api.Pod` literal was rewritten once as the
// pod's JSON, and each expected row as its READY, STATUS and RESTARTS cells (`<unknown>` ages
// left out). The Go tests' unnamed containers keep their (empty) names.

//! kubectl v1.36's own pod printer cases, run through [`PodSummary`] (E12-S02: the status
//! derivation is checked against the printer it ports, case by case).

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::{PodSummary, Resource};
use serde_json::{Value, json};

/// The printer's "now": `time.Now()` in the Go tests.
fn now() -> Timestamp {
    "2026-10-11T12:00:00Z".parse().expect("a timestamp")
}

/// `now` minus `secs`, as an RFC 3339 string.
fn ago(secs: i64) -> String {
    (now() - SignedDuration::from_secs(secs)).to_string()
}

const DAY: i64 = 24 * 3600;

fn pod(name: &str, spec: Value, status: Value, deleting: bool) -> Resource {
    let mut meta = json!({"name": name, "namespace": "default"});
    if deleting {
        meta["deletionTimestamp"] = json!(ago(10));
    }
    Resource::from_json(
        json!({"apiVersion": "v1", "kind": "Pod", "metadata": meta, "spec": spec, "status": status}),
    )
    .expect("a pod")
}

/// `make([]api.Container, n)`: `n` containers without names.
fn unnamed(n: usize) -> Value {
    Value::Array(vec![json!({}); n])
}

fn spec(inits: usize, containers: usize) -> Value {
    json!({"initContainers": unnamed(inits), "containers": unnamed(containers)})
}

/// The sidecar spec of the restartable-init-container tests.
fn sidecar_spec() -> Value {
    json!({
        "initContainers": [
            {"name": "restartable-init-1", "restartPolicy": "Always"},
            {"name": "restartable-init-2", "restartPolicy": "Always"}
        ],
        "containers": unnamed(1)
    })
}

fn last_terminated(secs: i64) -> Value {
    json!({"terminated": {"finishedAt": ago(secs)}})
}

struct Case {
    pod: Resource,
    /// READY, STATUS, RESTARTS as `kubectl get pods` prints them.
    want: (&'static str, &'static str, &'static str),
}

fn case(pod: Resource, want: (&'static str, &'static str, &'static str)) -> Case {
    Case { pod, want }
}

fn test_print_pod() -> Vec<Case> {
    let running = json!({"running": {}});
    vec![
        // test1: name, number of containers, restarts, ready status.
        case(
            pod(
                "test1",
                spec(0, 2),
                json!({"phase": "podPhase", "containerStatuses": [
                    {"ready": true, "restartCount": 3, "state": running},
                    {"restartCount": 3}
                ]}),
                false,
            ),
            ("1/2", "podPhase", "6"),
        ),
        // test2: a container error overwrites the pod phase.
        case(
            pod(
                "test2",
                spec(0, 2),
                json!({"phase": "podPhase", "containerStatuses": [
                    {"ready": true, "restartCount": 3, "state": running},
                    {"restartCount": 3, "state": {"waiting": {"reason": "ContainerWaitingReason"}}}
                ]}),
                false,
            ),
            ("1/2", "ContainerWaitingReason", "6"),
        ),
        // test3: the first container overwrites the rest.
        case(
            pod(
                "test3",
                spec(0, 2),
                json!({"phase": "podPhase", "containerStatuses": [
                    {"restartCount": 3, "state": {"waiting": {"reason": "ContainerWaitingReason"}}},
                    {"restartCount": 3, "state": {"terminated": {"reason": "ContainerTerminatedReason"}}}
                ]}),
                false,
            ),
            ("0/2", "ContainerWaitingReason", "6"),
        ),
        // test4: ready is not enough for reporting running.
        case(
            pod(
                "test4",
                spec(0, 2),
                json!({"phase": "podPhase", "containerStatuses": [
                    {"ready": true, "restartCount": 3, "state": running},
                    {"ready": true, "restartCount": 3}
                ]}),
                false,
            ),
            ("1/2", "podPhase", "6"),
        ),
        // test5: the pod reason wins over the phase.
        case(
            pod(
                "test5",
                spec(0, 2),
                json!({"reason": "podReason", "phase": "podPhase",
                "containerStatuses": [
                    {"ready": true, "restartCount": 3, "state": running},
                    {"ready": true, "restartCount": 3}
                ]}),
                false,
            ),
            ("1/2", "podReason", "6"),
        ),
        // test6: one running, one completed, no Ready condition.
        case(
            pod(
                "test6",
                spec(0, 2),
                json!({"phase": "Running", "containerStatuses": [
                    {"ready": true, "restartCount": 3, "state": {"terminated": {"reason": "Completed", "exitCode": 0}}},
                    {"ready": true, "restartCount": 3, "state": running}
                ]}),
                false,
            ),
            ("1/2", "NotReady", "6"),
        ),
        // test6 (second): the same with the Ready condition.
        case(
            pod(
                "test6",
                spec(0, 2),
                json!({"phase": "Running", "containerStatuses": [
                {"ready": true, "restartCount": 3, "state": {"terminated": {"reason": "Completed", "exitCode": 0}}},
                {"ready": true, "restartCount": 3, "state": running}
            ], "conditions": [{"type": "Ready", "status": "True"}]}),
                false,
            ),
            ("1/2", "Running", "6"),
        ),
        // test7: one init container restarting, one container not running.
        case(
            pod(
                "test7",
                spec(1, 1),
                json!({"phase": "podPhase",
                    "initContainerStatuses": [
                        {"ready": false, "restartCount": 3, "state": running, "lastState": last_terminated(10)}
                    ],
                    "containerStatuses": [{"ready": false, "restartCount": 0, "state": {"waiting": {}}}]
                }),
                false,
            ),
            ("0/1", "Init:0/1", "3 (10s ago)"),
        ),
        // test8: two init containers, one restarting, the other not running.
        case(
            pod(
                "test8",
                spec(2, 1),
                json!({"phase": "podPhase",
                    "initContainerStatuses": [
                        {"ready": false, "restartCount": 3, "state": running, "lastState": last_terminated(10)},
                        {"ready": false, "state": {"waiting": {}}}
                    ],
                    "containerStatuses": [{"ready": false, "state": {"waiting": {}}}]
                }),
                false,
            ),
            ("0/1", "Init:0/2", "3 (10s ago)"),
        ),
        // test9: one init container completed without restarts, the other restarting.
        case(
            pod(
                "test9",
                spec(2, 1),
                json!({"phase": "podPhase",
                    "initContainerStatuses": [
                        {"ready": false, "state": {"terminated": {}}},
                        {"ready": false, "restartCount": 3, "state": running, "lastState": last_terminated(10)}
                    ],
                    "containerStatuses": [{"ready": false, "state": {"waiting": {}}}]
                }),
                false,
            ),
            ("0/1", "Init:1/2", "3 (10s ago)"),
        ),
        // test10: one init container completed with restarts, the other restarting.
        case(
            pod(
                "test10",
                spec(2, 1),
                json!({"phase": "podPhase",
                    "initContainerStatuses": [
                        {"ready": false, "restartCount": 2, "state": {"terminated": {}}, "lastState": last_terminated(120)},
                        {"ready": false, "restartCount": 3, "state": running, "lastState": last_terminated(10)}
                    ],
                    "containerStatuses": [{"ready": false, "state": {"waiting": {}}}]
                }),
                false,
            ),
            ("0/1", "Init:1/2", "5 (10s ago)"),
        ),
        // test11: the init container completed with restarts, the container restarting.
        case(
            pod(
                "test11",
                spec(1, 1),
                json!({"phase": "Running",
                    "initContainerStatuses": [
                        {"ready": false, "restartCount": 2, "state": {"terminated": {}}, "lastState": last_terminated(120)}
                    ],
                    "containerStatuses": [
                        {"ready": false, "restartCount": 4, "state": running, "lastState": last_terminated(20)}
                    ]
                }),
                false,
            ),
            ("0/1", "Running", "4 (20s ago)"),
        ),
        // test12: one container that restarted 5d ago.
        case(
            pod(
                "test12",
                spec(0, 1),
                json!({"phase": "Running", "containerStatuses": [
                    {"ready": true, "restartCount": 3, "state": running, "lastState": last_terminated(5 * DAY)}
                ]}),
                false,
            ),
            ("1/1", "Running", "3 (5d ago)"),
        ),
        // test13: one never restarted, the other restarted 10d ago.
        case(
            pod(
                "test13",
                spec(0, 2),
                json!({"phase": "Running", "containerStatuses": [
                    {"ready": true, "restartCount": 0, "state": running},
                    {"ready": true, "restartCount": 3, "state": running, "lastState": last_terminated(10 * DAY)}
                ]}),
                false,
            ),
            ("2/2", "Running", "3 (10d ago)"),
        ),
        // test14: restarted 5d and 20d ago: the latest wins.
        case(
            pod(
                "test14",
                spec(0, 2),
                json!({"phase": "Running", "containerStatuses": [
                    {"ready": true, "restartCount": 6, "state": running, "lastState": last_terminated(5 * DAY)},
                    {"ready": true, "restartCount": 3, "state": running, "lastState": last_terminated(20 * DAY)}
                ]}),
                false,
            ),
            ("2/2", "Running", "9 (5d ago)"),
        ),
        // test15: PodScheduled with reason SchedulingGated.
        case(
            pod(
                "test15",
                spec(0, 2),
                json!({"phase": "podPhase", "conditions": [
                    {"type": "PodScheduled", "status": "False", "reason": "SchedulingGated"}
                ]}),
                false,
            ),
            ("0/2", "SchedulingGated", "0"),
        ),
        // test16: succeeded.
        case(
            pod(
                "test16",
                spec(0, 1),
                json!({"phase": "Succeeded", "containerStatuses": [
                    {"ready": false, "state": {"terminated": {"reason": "Completed", "exitCode": 0}}}
                ]}),
                false,
            ),
            ("0/1", "Completed", "0"),
        ),
        // test17: failed.
        case(
            pod(
                "test17",
                spec(0, 1),
                json!({"phase": "Failed", "containerStatuses": [
                    {"ready": false, "state": {"terminated": {"reason": "Error", "exitCode": 1}}}
                ]}),
                false,
            ),
            ("0/1", "Error", "0"),
        ),
        // test18: succeeded and being deleted: still Completed.
        case(
            pod(
                "test18",
                spec(0, 1),
                json!({"phase": "Succeeded", "containerStatuses": [
                    {"ready": false, "state": {"terminated": {"reason": "Completed", "exitCode": 0}}}
                ]}),
                true,
            ),
            ("0/1", "Completed", "0"),
        ),
        // test19: running and being deleted.
        case(
            pod(
                "test19",
                spec(0, 1),
                json!({"phase": "Running", "containerStatuses": [
                    {"ready": false, "state": running}
                ]}),
                true,
            ),
            ("0/1", "Terminating", "0"),
        ),
        // test20: pending and being deleted.
        case(
            pod("test20", spec(0, 1), json!({"phase": "Pending"}), true),
            ("0/1", "Terminating", "0"),
        ),
        // test21: Error then Running (kubernetes#107713).
        case(
            pod(
                "test21",
                spec(0, 2),
                json!({"phase": "Failed", "containerStatuses": [
                    {"ready": false, "state": {"terminated": {"finishedAt": ago(0), "exitCode": 1, "reason": "podCrash"}}},
                    {"ready": true, "state": {"running": {"startedAt": ago(0)}}}
                ]}),
                false,
            ),
            ("1/2", "podCrash", "0"),
        ),
        // test22: Running then Error.
        case(
            pod(
                "test22",
                spec(0, 2),
                json!({"phase": "Failed", "containerStatuses": [
                    {"ready": true, "state": {"running": {"startedAt": ago(0)}}},
                    {"ready": false, "state": {"terminated": {"finishedAt": ago(0), "exitCode": 1, "reason": "podCrash"}}}
                ]}),
                false,
            ),
            ("1/2", "podCrash", "0"),
        ),
        // test23: Running, Error, Completed (all ready).
        case(
            pod(
                "test23",
                spec(0, 3),
                json!({"phase": "Failed", "containerStatuses": [
                    {"ready": true, "state": {"running": {"startedAt": ago(0)}}},
                    {"ready": true, "state": {"terminated": {"finishedAt": ago(0), "exitCode": 1, "reason": "podCrash"}}},
                    {"ready": true, "state": {"terminated": {"finishedAt": ago(0), "exitCode": 0}}}
                ]}),
                false,
            ),
            ("1/3", "podCrash", "0"),
        ),
        // test24: Running, Error, Completed (none ready).
        case(
            pod(
                "test24",
                spec(0, 3),
                json!({"phase": "Failed", "containerStatuses": [
                    {"ready": false, "state": {"running": {"startedAt": ago(0)}}},
                    {"ready": false, "state": {"terminated": {"finishedAt": ago(0), "exitCode": 1, "reason": "podCrash"}}},
                    {"ready": false, "state": {"terminated": {"finishedAt": ago(0), "exitCode": 0}}}
                ]}),
                false,
            ),
            ("0/3", "podCrash", "0"),
        ),
    ]
}

fn test_print_pod_with_restartable_init_container() -> Vec<Case> {
    let running = json!({"running": {}});
    vec![
        // Two sidecars, the first running but not started.
        case(
            pod(
                "test1",
                sidecar_spec(),
                json!({"phase": "Pending",
                    "initContainerStatuses": [
                        {"name": "restartable-init-1", "ready": false, "restartCount": 3, "state": running,
                         "started": false, "lastState": last_terminated(10)},
                        {"name": "restartable-init-2", "ready": false, "state": {"waiting": {}}, "started": false}
                    ],
                    "containerStatuses": [{"ready": false, "restartCount": 0, "state": {"waiting": {}}}],
                    "conditions": [{"type": "Initialized", "status": "False"}]
                }),
                false,
            ),
            ("0/3", "Init:0/2", "3 (10s ago)"),
        ),
        // The first sidecar started, the second running but not started.
        case(
            pod(
                "test1",
                sidecar_spec(),
                json!({"phase": "Pending",
                    "initContainerStatuses": [
                        {"name": "restartable-init-1", "ready": false, "restartCount": 3, "state": running,
                         "started": true, "lastState": last_terminated(10)},
                        {"name": "restartable-init-2", "ready": false, "state": running, "started": false}
                    ],
                    "containerStatuses": [{"ready": false, "restartCount": 0, "state": {"waiting": {}}}],
                    "conditions": [{"type": "Initialized", "status": "False"}]
                }),
                false,
            ),
            ("0/3", "Init:1/2", "3 (10s ago)"),
        ),
        // Both sidecars started, the container running.
        case(
            pod(
                "test2",
                sidecar_spec(),
                json!({"phase": "Running",
                    "initContainerStatuses": [
                        {"name": "restartable-init-1", "ready": false, "restartCount": 3, "state": running,
                         "started": true, "lastState": last_terminated(10)},
                        {"name": "restartable-init-2", "ready": false, "state": running, "started": true}
                    ],
                    "containerStatuses": [
                        {"ready": true, "restartCount": 4, "state": running, "lastState": last_terminated(20)}
                    ],
                    "conditions": [{"type": "Initialized", "status": "True"}]
                }),
                false,
            ),
            ("1/3", "Running", "7 (10s ago)"),
        ),
        // Both sidecars exited non-zero, the container completed (the pod's sidecar case).
        case(
            pod(
                "test3",
                sidecar_spec(),
                json!({"phase": "Succeeded",
                    "initContainerStatuses": [
                        {"name": "restartable-init-1", "ready": false, "restartCount": 3,
                         "state": {"terminated": {"reason": "Error", "exitCode": 137}},
                         "started": false, "lastState": last_terminated(10)},
                        {"name": "restartable-init-2", "ready": false,
                         "state": {"terminated": {"reason": "Error", "exitCode": 137}}, "started": false}
                    ],
                    "containerStatuses": [
                        {"ready": false, "restartCount": 4,
                         "state": {"terminated": {"reason": "Completed", "exitCode": 0}},
                         "lastState": last_terminated(20)}
                    ],
                    "conditions": [{"type": "Initialized", "status": "True"}]
                }),
                false,
            ),
            ("0/3", "Completed", "7 (10s ago)"),
        ),
        // Statuses for containers the spec does not have.
        case(
            pod(
                "test4",
                json!({
                    "initContainers": [
                        {"name": "init1", "image": "initimage"},
                        {"name": "sidecar1", "image": "sidecarimage", "restartPolicy": "Always"}
                    ],
                    "containers": [{"name": "container1", "image": "containerimage"}]
                }),
                json!({"phase": "Running",
                    "initContainerStatuses": [{"name": "initinvalid"}, {"name": "init1"}, {"name": "sidecar1"}],
                    "containerStatuses": [{"name": "containerinvalid"}, {"name": "container1"}]
                }),
                false,
            ),
            ("0/2", "Init:0/2", "0"),
        ),
    ]
}

fn check(cases: Vec<Case>, test: &str) {
    for (i, Case { pod, want }) in cases.into_iter().enumerate() {
        let s = PodSummary::from_resource(&pod).expect("a pod summary");
        let got = (
            s.ready_display(),
            s.status.to_string(),
            s.restarts_display(now()),
        );
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2.as_str()),
            want,
            "{test} case {i} ({})",
            pod.meta.name
        );
    }
}

#[test]
fn test_print_pod_v136() {
    check(test_print_pod(), "TestPrintPod");
}

#[test]
fn test_print_pod_with_restartable_init_container_v136() {
    check(
        test_print_pod_with_restartable_init_container(),
        "TestPrintPodWithRestartableInitContainer",
    );
}
