//! Which containers a shell can open, and the default.

use oxikube_domain::Resource;
use oxikube_domain::view::ContainerKind;
use serde_json::{Value, json};

use super::{annotated, pod_json};
use crate::exec::{DEFAULT_CONTAINER_ANNOTATION, PodContainers};

fn of(json: Value) -> PodContainers {
    PodContainers::of(&Resource::from_json(json).unwrap())
}

fn names(pod: &PodContainers) -> Vec<&str> {
    pod.containers().iter().map(|c| &*c.name).collect()
}

#[test]
fn regular_containers_are_candidates_with_their_state() {
    let pod = of(pod_json(&[("app", true), ("sidecar", false)]));
    assert_eq!(names(&pod), ["app", "sidecar"]);
    assert!(pod.containers()[0].running && !pod.containers()[1].running);
    assert_eq!(pod.containers()[0].label(), "app");
    assert_eq!(pod.containers()[1].label(), "sidecar (not running)");
}

#[test]
fn the_default_is_the_annotation_else_the_first_regular_container() {
    let plain = of(pod_json(&[("a", true), ("b", true)]));
    assert_eq!(&*plain.default_container().unwrap().name, "a");
    let named = of(annotated(
        pod_json(&[("a", true), ("b", true)]),
        DEFAULT_CONTAINER_ANNOTATION,
        "b",
    ));
    assert_eq!(&*named.default_container().unwrap().name, "b");
    let stale = of(annotated(
        pod_json(&[("a", true)]),
        DEFAULT_CONTAINER_ANNOTATION,
        "gone",
    ));
    assert_eq!(
        &*stale.default_container().unwrap().name,
        "a",
        "an annotation naming no container is ignored"
    );
}

#[test]
fn init_containers_count_only_while_they_run_and_ephemeral_ones_do() {
    let mut json = pod_json(&[("app", true)]);
    json["spec"]["initContainers"] = json!([
        {"name": "migrate", "image": "busybox"},
        {"name": "wait", "image": "busybox"},
    ]);
    json["status"]["initContainerStatuses"] = json!([
        {"name": "migrate", "ready": true, "restartCount": 0, "image": "busybox",
         "state": {"terminated": {"exitCode": 0, "reason": "Completed"}}},
        {"name": "wait", "ready": false, "restartCount": 0, "image": "busybox",
         "state": {"running": {"startedAt": "2026-10-01T00:00:00Z"}}},
    ]);
    json["spec"]["ephemeralContainers"] = json!([{"name": "debugger-1", "image": "busybox"}]);
    json["status"]["ephemeralContainerStatuses"] = json!([
        {"name": "debugger-1", "ready": false, "restartCount": 0, "image": "busybox",
         "state": {"running": {"startedAt": "2026-10-01T00:00:00Z"}}},
    ]);
    let pod = of(json);
    assert_eq!(names(&pod), ["wait", "app", "debugger-1"]);
    let kinds: Vec<_> = pod.containers().iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            ContainerKind::Init,
            ContainerKind::Regular,
            ContainerKind::Ephemeral
        ]
    );
    assert_eq!(pod.containers()[0].label(), "wait (init)");
    assert_eq!(pod.containers()[2].label(), "debugger-1 (ephemeral)");
    assert!(
        !pod.contains("migrate"),
        "a finished init container is not offered"
    );
    assert_eq!(
        &*pod.default_container().unwrap().name,
        "app",
        "the default is a regular container, not an init one"
    );
}

#[test]
fn windows_pods_are_recognised() {
    let mut json = pod_json(&[("app", true)]);
    assert!(!of(json.clone()).is_windows());
    json["spec"]["os"] = json!({"name": "windows"});
    assert!(of(json).is_windows());
    let mut selector = pod_json(&[("app", true)]);
    selector["spec"]["nodeSelector"] = json!({"kubernetes.io/os": "windows"});
    assert!(of(selector).is_windows());
}

#[test]
fn something_that_is_not_a_pod_has_no_containers() {
    let config_map = json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "c", "namespace": "default", "uid": "u"},
    });
    assert!(of(config_map).containers().is_empty());
}
