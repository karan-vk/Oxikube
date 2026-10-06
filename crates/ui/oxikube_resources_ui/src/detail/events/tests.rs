use std::sync::Arc;

use oxikube_app::store::StoreObject;
use oxikube_domain::Resource;
use oxikube_domain::event::EventType;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use serde_json::{Value, json};

use super::*;

fn cluster() -> ClusterId {
    ClusterId::new("/k", &ContextName::new("kind"))
}

fn event(
    name: &str,
    object: &str,
    kind: &str,
    uid: &str,
    last: &str,
    ty: &str,
) -> Arc<StoreObject> {
    let json: Value = json!({
        "apiVersion": "v1", "kind": "Event",
        "metadata": {"name": name, "namespace": "demo", "uid": name},
        "involvedObject": {"apiVersion": "v1", "kind": kind, "name": object,
                           "namespace": "demo", "uid": uid},
        "type": ty, "reason": "Reason", "message": format!("about {object}"),
        "count": 2, "lastTimestamp": last,
        "source": {"component": "kubelet"}
    });
    Arc::new(StoreObject::Resource(Resource::from_json(json).unwrap()))
}

fn target() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "demo", "web-0")
}

#[test]
fn keeps_only_the_events_about_the_object_newest_first() {
    let all = vec![
        event("e1", "web-0", "Pod", "u1", "2026-01-01T00:00:00Z", "Normal"),
        event("e2", "web-1", "Pod", "u2", "2026-01-01T00:05:00Z", "Normal"),
        event(
            "e3",
            "web-0",
            "Pod",
            "u1",
            "2026-01-01T00:10:00Z",
            "Warning",
        ),
        event(
            "e4",
            "web-0",
            "Service",
            "u3",
            "2026-01-01T00:20:00Z",
            "Normal",
        ),
    ];
    let rows = events_about(&cluster(), &target(), Some("u1"), &all);
    assert_eq!(
        rows.len(),
        2,
        "web-1 and the Service named web-0 are other objects"
    );
    assert_eq!(rows[0].kind, EventType::Warning, "newest first");
    assert_eq!(rows[0].message, "about web-0");
    assert_eq!(rows[0].count, 2);
    assert_eq!(rows[0].source.as_deref(), Some("kubelet"));
}

#[test]
fn an_earlier_object_of_the_same_name_is_not_this_one() {
    let all = vec![
        event(
            "old",
            "web-0",
            "Pod",
            "old-uid",
            "2026-01-01T00:00:00Z",
            "Normal",
        ),
        event(
            "new",
            "web-0",
            "Pod",
            "new-uid",
            "2026-01-02T00:00:00Z",
            "Normal",
        ),
    ];
    let rows = events_about(&cluster(), &target(), Some("new-uid"), &all);
    assert_eq!(rows.len(), 1);
    // Without a uid to compare (the object has none yet), name and kind decide.
    assert_eq!(events_about(&cluster(), &target(), None, &all).len(), 2);
}

#[test]
fn the_list_is_capped() {
    let all: Vec<_> = (0..MAX_EVENTS + 50)
        .map(|i| {
            event(
                &format!("e{i}"),
                "web-0",
                "Pod",
                "u1",
                "2026-01-01T00:00:00Z",
                "Normal",
            )
        })
        .collect();
    assert_eq!(
        events_about(&cluster(), &target(), None, &all).len(),
        MAX_EVENTS
    );
}

#[test]
fn a_nodes_events_carry_the_node_name_as_uid() {
    let node = ResourceRef::cluster_scoped(cluster(), Gvk::new("", "v1", "Node"), "kind-cp");
    let mut json = json!({
        "apiVersion": "v1", "kind": "Event",
        "metadata": {"name": "kind-cp.1", "namespace": "default", "uid": "e1"},
        "involvedObject": {"kind": "Node", "name": "kind-cp", "uid": "kind-cp"},
        "type": "Normal", "reason": "NodeReady", "message": "ready",
        "lastTimestamp": "2026-01-01T00:00:00Z"
    });
    let ready = Arc::new(StoreObject::Resource(
        Resource::from_json(json.clone()).unwrap(),
    ));
    json["metadata"]["name"] = json!("kind-cp.2");
    json["involvedObject"]["uid"] = json!("someone-elses-uid");
    let other = Arc::new(StoreObject::Resource(Resource::from_json(json).unwrap()));
    let rows = events_about(
        &cluster(),
        &node,
        Some("3f2c9a52-real-node-uid"),
        &[ready, other],
    );
    assert_eq!(
        rows.len(),
        1,
        "the kubelet's event is the node's, the other uid is not"
    );
    assert_eq!(&*rows[0].reason, "NodeReady");
}
