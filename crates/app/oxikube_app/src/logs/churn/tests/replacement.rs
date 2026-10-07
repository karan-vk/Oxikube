//! `find_replacement`: the pod that took over, by controller (Deployment rollout, StatefulSet,
//! DaemonSet, Job) or name, and none when nothing took over.

use futures::executor::block_on;
use oxikube_domain::Resource;
use oxikube_domain::ids::Gvk;
use oxikube_testkit::{FakeResourcePort, daemonset, deployment, replicaset, statefulset};
use serde_json::{Value, json};

use super::{owned_pod, resource};
use crate::logs::{PodIdentity, find_replacement};

fn owner_ref(json: &mut Value, kind: &str, name: &str) {
    json["metadata"]["ownerReferences"] = json!([{
        "apiVersion": "apps/v1", "kind": kind, "name": name,
        "uid": format!("uid-{name}"), "controller": true
    }]);
}

/// A ReplicaSet `name` of the `web` Deployment.
fn web_rs(name: &str) -> Resource {
    let mut rs = replicaset()
        .name(name)
        .namespace("default")
        .label("app", "web")
        .json();
    owner_ref(&mut rs, "Deployment", "web");
    resource(rs)
}

fn pod(name: &str, uid: &str, owner: (&str, &str), created: &str) -> Value {
    let mut pod = owned_pod(name, uid, Some(owner), "Running");
    pod["metadata"]["creationTimestamp"] = json!(created);
    pod
}

fn on_node(mut pod: Value, node: &str) -> Value {
    pod["spec"]["nodeName"] = json!(node);
    pod
}

fn terminating(mut pod: Value) -> Value {
    pod["metadata"]["deletionTimestamp"] = json!("2026-10-07T12:10:00Z");
    pod
}

fn find(cluster: &FakeResourcePort, gone: &PodIdentity) -> Option<String> {
    block_on(find_replacement(cluster, gone)).expect("the lookup reads")
}

fn pod_kind() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

#[test]
fn a_deployment_rollout_is_followed_to_the_new_replica_sets_newest_pod() {
    let cluster = FakeResourcePort::new();
    let old = pod(
        "web-5d8-a",
        "u1",
        ("ReplicaSet", "web-5d8"),
        "2026-10-07T11:00:00Z",
    );
    let gone = PodIdentity::of(&resource(old));
    cluster.insert(
        deployment()
            .name("web")
            .namespace("default")
            .label("app", "web")
            .build(),
    );
    cluster.insert(web_rs("web-5d8"));
    cluster.insert(web_rs("web-7f9"));
    // The old pod is gone; its sibling of the old ReplicaSet is terminating; two new pods run.
    cluster.insert(resource(terminating(pod(
        "web-5d8-c",
        "u3",
        ("ReplicaSet", "web-5d8"),
        "2026-10-07T11:00:00Z",
    ))));
    cluster.insert(resource(pod(
        "web-7f9-x",
        "u4",
        ("ReplicaSet", "web-7f9"),
        "2026-10-07T12:00:01Z",
    )));
    cluster.insert(resource(pod(
        "web-7f9-y",
        "u5",
        ("ReplicaSet", "web-7f9"),
        "2026-10-07T12:00:05Z",
    )));
    assert_eq!(find(&cluster, &gone).as_deref(), Some("web-7f9-y"));
}

#[test]
fn a_statefulset_pod_is_followed_to_its_namesake() {
    let cluster = FakeResourcePort::new();
    let owner = ("StatefulSet", "web");
    let gone = PodIdentity::of(&resource(pod("web-0", "u1", owner, "2026-10-07T11:00:00Z")));
    cluster.insert(
        statefulset()
            .name("web")
            .namespace("default")
            .label("app", "web")
            .build(),
    );
    cluster.insert(resource(pod("web-1", "u7", owner, "2026-10-07T12:00:09Z")));
    assert_eq!(
        find(&cluster, &gone),
        None,
        "before the namesake exists, a sibling ordinal is not its replacement"
    );
    cluster.insert(resource(pod("web-0", "u2", owner, "2026-10-07T12:00:00Z")));
    assert_eq!(find(&cluster, &gone).as_deref(), Some("web-0"));
}

#[test]
fn a_daemonset_pod_is_followed_on_its_node() {
    let cluster = FakeResourcePort::new();
    let owner = ("DaemonSet", "agent");
    let gone = PodIdentity::of(&resource(on_node(
        pod("agent-a1", "u1", owner, "2026-10-07T11:00:00Z"),
        "node-a",
    )));
    let mut ds = daemonset()
        .name("agent")
        .namespace("default")
        .label("app", "web")
        .json();
    ds["metadata"]["uid"] = json!("uid-agent");
    cluster.insert(resource(ds));
    cluster.insert(resource(on_node(
        pod("agent-b9", "u3", owner, "2026-10-07T12:00:30Z"),
        "node-b",
    )));
    assert_eq!(
        find(&cluster, &gone),
        None,
        "before the node has its new pod, a pod on another node is not its replacement"
    );
    cluster.insert(resource(on_node(
        pod("agent-a2", "u2", owner, "2026-10-07T12:00:00Z"),
        "node-a",
    )));
    assert_eq!(find(&cluster, &gone).as_deref(), Some("agent-a2"));
}

#[test]
fn nothing_takes_over_from_a_deleted_bare_pod_or_a_deleted_controller() {
    let cluster = FakeResourcePort::new();
    let bare = PodIdentity::of(&resource(owned_pod("solo", "u1", None, "Running")));
    assert_eq!(find(&cluster, &bare), None);
    // The same name made again by hand is its replacement.
    cluster.insert(resource(owned_pod("solo", "u2", None, "Running")));
    assert_eq!(find(&cluster, &bare).as_deref(), Some("solo"));
    // Still the gone pod itself (same uid): not a replacement.
    cluster.remove(&pod_kind(), Some("default"), "solo");
    cluster.insert(resource(owned_pod("solo", "u1", None, "Running")));
    assert_eq!(find(&cluster, &bare), None);

    let orphan = PodIdentity::of(&resource(owned_pod(
        "web-5d8-a",
        "u1",
        Some(("ReplicaSet", "web-5d8")),
        "Running",
    )));
    assert_eq!(find(&cluster, &orphan), None, "the ReplicaSet is gone too");
}

#[test]
fn a_replica_set_without_a_deployment_uses_its_own_selector() {
    let cluster = FakeResourcePort::new();
    let gone = PodIdentity::of(&resource(pod(
        "web-5d8-a",
        "u1",
        ("ReplicaSet", "web-5d8"),
        "2026-10-07T11:00:00Z",
    )));
    cluster.insert(
        replicaset()
            .name("web-5d8")
            .namespace("default")
            .label("app", "web")
            .build(),
    );
    cluster.insert(resource(pod(
        "web-5d8-b",
        "u2",
        ("ReplicaSet", "web-5d8"),
        "2026-10-07T12:00:00Z",
    )));
    assert_eq!(find(&cluster, &gone).as_deref(), Some("web-5d8-b"));
}

#[test]
fn a_replica_left_after_a_scale_down_is_not_a_replacement() {
    let cluster = FakeResourcePort::new();
    let owner = ("ReplicaSet", "web-5d8");
    let gone = PodIdentity::of(&resource(pod(
        "web-5d8-c",
        "u3",
        owner,
        "2026-10-07T11:00:05Z",
    )));
    cluster.insert(
        deployment()
            .name("web")
            .namespace("default")
            .label("app", "web")
            .build(),
    );
    cluster.insert(web_rs("web-5d8"));
    // The survivors of the scale-down are older than the gone pod (or as old): siblings.
    cluster.insert(resource(pod(
        "web-5d8-a",
        "u1",
        owner,
        "2026-10-07T11:00:00Z",
    )));
    cluster.insert(resource(pod(
        "web-5d8-b",
        "u2",
        owner,
        "2026-10-07T11:00:05Z",
    )));
    assert_eq!(find(&cluster, &gone), None);
    // A pod the ReplicaSet makes after it is one.
    cluster.insert(resource(pod(
        "web-5d8-d",
        "u4",
        owner,
        "2026-10-07T12:00:00Z",
    )));
    assert_eq!(find(&cluster, &gone).as_deref(), Some("web-5d8-d"));
}

#[test]
fn a_job_retry_is_the_newer_pod_of_the_job() {
    let cluster = FakeResourcePort::new();
    let owner = ("Job", "batch");
    let gone = PodIdentity::of(&resource(pod(
        "batch-a",
        "u1",
        owner,
        "2026-10-07T11:00:00Z",
    )));
    let mut job = oxikube_testkit::job()
        .name("batch")
        .namespace("default")
        .label("app", "web")
        .json();
    job["metadata"]["uid"] = json!("uid-batch");
    cluster.insert(resource(job));
    cluster.insert(resource(pod(
        "batch-b",
        "u2",
        owner,
        "2026-10-07T10:59:00Z",
    )));
    assert_eq!(
        find(&cluster, &gone),
        None,
        "an older parallel pod is not the retry"
    );
    cluster.insert(resource(pod(
        "batch-c",
        "u3",
        owner,
        "2026-10-07T11:00:40Z",
    )));
    assert_eq!(find(&cluster, &gone).as_deref(), Some("batch-c"));
}
