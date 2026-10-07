//! `ExecService` against the testkit fakes (a real session manager over a fake connector, so the
//! service reaches the `FakeExecPort` and the pod through `ClusterPorts`, like the app does).

mod containers;
mod open;
mod plan;
mod shell;

use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::ids::ResourceRef;
use oxikube_ports::ExitStatus;
use oxikube_testkit::{FakeExecPort, FakeResourcePort, FakeTerminalBackend};
use serde_json::{Value, json};

use super::ExecService;
use crate::testing::{Harness, id, pod};

/// The service over cluster `a`, with its fake exec port and resource port.
pub(super) struct Fixture {
    pub h: Harness,
    pub service: ExecService,
    pub exec: Arc<FakeExecPort>,
    pub resources: Arc<FakeResourcePort>,
}

impl Fixture {
    pub fn new() -> Self {
        let h = Harness::new();
        h.connect("a", false);
        let ports = h.connector.ports_for(&id("a"));
        Self {
            service: ExecService::new(h.manager.clone()),
            exec: ports.exec,
            resources: ports.resources,
            h,
        }
    }

    /// `web-0` in `default`.
    pub fn pod(&self) -> ResourceRef {
        pod("a", "web-0")
    }

    /// The pod `web-0` as the cluster returns it.
    pub fn serve(&self, json: Value) {
        self.resources
            .script()
            .get
            .push_ok(Resource::from_json(json).expect("a valid pod"));
    }
}

/// A pod `web-0` with `containers` (name, running) in `spec.containers`.
pub(super) fn pod_json(containers: &[(&str, bool)]) -> Value {
    let specs: Vec<Value> = containers
        .iter()
        .map(|(name, _)| json!({"name": name, "image": "busybox"}))
        .collect();
    let statuses: Vec<Value> = containers
        .iter()
        .map(|(name, running)| {
            let state = if *running {
                json!({"running": {"startedAt": "2026-10-01T00:00:00Z"}})
            } else {
                json!({"waiting": {"reason": "CrashLoopBackOff"}})
            };
            json!({"name": name, "ready": *running, "restartCount": 0, "image": "busybox", "state": state})
        })
        .collect();
    json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "web-0", "namespace": "default", "uid": "u1"},
        "spec": {"containers": specs},
        "status": {"phase": "Running", "containerStatuses": statuses},
    })
}

/// `pod_json` with an annotation.
pub(super) fn annotated(mut pod: Value, key: &str, value: &str) -> Value {
    pod["metadata"]["annotations"] = json!({ key: value });
    pod
}

/// A backend whose probe ended with `code`.
pub(super) fn probe_exit(code: i32) -> FakeTerminalBackend {
    let backend = FakeTerminalBackend::silent();
    backend.exit(if code == 0 {
        ExitStatus::success()
    } else {
        ExitStatus::with_code(code)
    });
    backend
}
