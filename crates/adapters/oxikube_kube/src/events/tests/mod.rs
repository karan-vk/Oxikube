//! Events feed tests: the ring (`ring`), the merge pump over scripted event APIs (`feed`),
//! and the storm (`perf`).

mod feed;
mod perf;
mod ring;

use oxikube_domain::event::Event;
use oxikube_domain::ids::{ClusterId, ContextName};
use serde_json::{Value, json};

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("kubeconfig", &ContextName::new("test"))
}

/// A `core/v1` event object about pod `regarding` (`regarding-uid` is `uid-of-<regarding>`).
pub(super) fn core_event(uid: &str, regarding: &str, last: &str, count: u32) -> Value {
    json!({
        "apiVersion": "v1", "kind": "Event",
        "metadata": {
            "name": format!("{regarding}.{uid}"), "namespace": "default", "uid": uid,
            "resourceVersion": "10", "creationTimestamp": "2026-10-03T11:00:00Z",
            "managedFields": [{"manager": "kubelet", "operation": "Update"}],
        },
        "involvedObject": {"kind": "Pod", "namespace": "default", "name": regarding,
            "apiVersion": "v1", "uid": format!("uid-of-{regarding}")},
        "reason": "BackOff", "message": "Back-off restarting failed container",
        "source": {"component": "kubelet", "host": "node-1"},
        "firstTimestamp": "2026-10-03T11:00:00Z", "lastTimestamp": last,
        "count": count, "type": "Warning",
    })
}

/// The `events.k8s.io/v1` view of the same stored event as [`core_event`].
pub(super) fn v1_event(uid: &str, regarding: &str, last: &str, count: u32) -> Value {
    json!({
        "apiVersion": "events.k8s.io/v1", "kind": "Event",
        "metadata": {
            "name": format!("{regarding}.{uid}"), "namespace": "default", "uid": uid,
            "resourceVersion": "10", "creationTimestamp": "2026-10-03T11:00:00Z",
        },
        "eventTime": null,
        "regarding": {"kind": "Pod", "namespace": "default", "name": regarding,
            "apiVersion": "v1", "uid": format!("uid-of-{regarding}")},
        "reason": "BackOff", "note": "Back-off restarting failed container",
        "reportingController": "kubelet", "reportingInstance": "node-1",
        "deprecatedFirstTimestamp": "2026-10-03T11:00:00Z", "deprecatedLastTimestamp": last,
        "deprecatedCount": count, "type": "Warning",
    })
}

/// The domain event of a `core/v1` fixture.
pub(super) fn domain(json: &Value) -> Event {
    Event::from_json(&cluster(), json).expect("fixture maps")
}
