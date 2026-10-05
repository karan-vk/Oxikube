//! Reading a container's readiness off a pod.

use k8s_openapi::api::core::v1::Pod;
use serde_json::{Value, json};

use crate::remote::exec::wait::{Container, Readiness, readiness};

fn pod(status: &Value) -> Pod {
    serde_json::from_value(json!({"metadata": {"name": "p"}, "status": status})).expect("pod")
}

fn regular() -> Container {
    Container::Regular("shell".into())
}

#[test]
fn running_is_ready() {
    let pod = pod(&json!({"phase": "Running", "containerStatuses": [
        {"name": "shell", "ready": true, "restartCount": 0, "image": "i", "imageID": "",
         "state": {"running": {}}}]}));
    assert_eq!(readiness(&pod, &regular()), Readiness::Running);
}

#[test]
fn pending_and_unreported_containers_keep_waiting() {
    assert_eq!(
        readiness(&pod(&json!({"phase": "Pending"})), &regular()),
        Readiness::Waiting
    );
    let creating = pod(&json!({"phase": "Pending", "containerStatuses": [
        {"name": "shell", "ready": false, "restartCount": 0, "image": "i", "imageID": "",
         "state": {"waiting": {"reason": "ContainerCreating"}}}]}));
    assert_eq!(readiness(&creating, &regular()), Readiness::Waiting);
}

#[test]
fn an_image_that_cannot_be_pulled_fails_at_once() {
    let pod = pod(&json!({"phase": "Pending", "containerStatuses": [
        {"name": "shell", "ready": false, "restartCount": 0, "image": "i", "imageID": "",
         "state": {"waiting": {"reason": "ImagePullBackOff", "message": "Back-off pulling image \"nope\""}}}]}));
    let Readiness::Failed(reason) = readiness(&pod, &regular()) else {
        panic!("expected a failure");
    };
    assert!(reason.starts_with("ImagePullBackOff: "), "{reason}");
}

#[test]
fn a_container_that_exited_or_a_pod_that_ended_fails() {
    let exited = pod(&json!({"phase": "Running", "containerStatuses": [
        {"name": "shell", "ready": false, "restartCount": 0, "image": "i", "imageID": "",
         "state": {"terminated": {"exitCode": 1}}}]}));
    assert!(matches!(readiness(&exited, &regular()), Readiness::Failed(r) if r.contains("code 1")));
    let failed = pod(&json!({"phase": "Failed"}));
    assert!(matches!(
        readiness(&failed, &regular()),
        Readiness::Failed(_)
    ));
}

#[test]
fn a_terminating_pod_fails() {
    let pod: Pod = serde_json::from_value(json!({
        "metadata": {"name": "p", "deletionTimestamp": "2026-01-01T00:00:00Z"},
        "status": {"phase": "Running"},
    }))
    .expect("pod");
    assert!(
        matches!(readiness(&pod, &regular()), Readiness::Failed(r) if r.contains("terminating"))
    );
}

#[test]
fn ephemeral_containers_are_read_from_their_own_status_list() {
    let pod = pod(&json!({"phase": "Running",
        "containerStatuses": [{"name": "dbg", "ready": true, "restartCount": 0, "image": "i", "imageID": "",
                               "state": {"running": {}}}],
        "ephemeralContainerStatuses": [{"name": "dbg", "ready": false, "restartCount": 0, "image": "i", "imageID": "",
                                        "state": {"waiting": {"reason": "ContainerCreating"}}}]}));
    assert_eq!(
        readiness(&pod, &Container::Ephemeral("dbg".into())),
        Readiness::Waiting
    );
    assert_eq!(
        readiness(&pod, &Container::Regular("dbg".into())),
        Readiness::Running
    );
}
