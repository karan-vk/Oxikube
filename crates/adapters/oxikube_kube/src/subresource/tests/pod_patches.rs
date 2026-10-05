//! Ephemeral container and resize bodies.

use oxikube_ports::PatchKind;
use serde_json::json;

use crate::subresource::{
    EphemeralContainerSpec, ResizeSpec, ephemeral_container_patch, resize_patch,
};

#[test]
fn an_ephemeral_container_patch_carries_only_what_was_set() {
    let minimal = ephemeral_container_patch(&EphemeralContainerSpec {
        name: "dbg".into(),
        image: "busybox:1.36".into(),
        ..EphemeralContainerSpec::default()
    });
    assert_eq!(minimal.kind, PatchKind::Strategic);
    assert_eq!(
        minimal.body,
        json!({"spec": {"ephemeralContainers": [{"name": "dbg", "image": "busybox:1.36"}]}})
    );

    let full = ephemeral_container_patch(&EphemeralContainerSpec {
        name: "dbg".into(),
        image: "busybox:1.36".into(),
        command: vec!["sh".into()],
        target_container: Some("app".into()),
        stdin: true,
        tty: true,
    });
    assert_eq!(
        full.body,
        json!({"spec": {"ephemeralContainers": [{
            "name": "dbg", "image": "busybox:1.36", "command": ["sh"],
            "targetContainerName": "app", "stdin": true, "tty": true,
        }]}})
    );
}

#[test]
fn a_resize_patch_names_the_container_and_the_lists_given() {
    let patch = resize_patch(&ResizeSpec {
        container: "app".into(),
        requests: vec![("cpu".into(), "500m".into())],
        limits: vec![
            ("cpu".into(), "1".into()),
            ("memory".into(), "256Mi".into()),
        ],
    });
    assert_eq!(patch.kind, PatchKind::Strategic);
    assert_eq!(
        patch.body,
        json!({"spec": {"containers": [{"name": "app", "resources": {
            "requests": {"cpu": "500m"},
            "limits": {"cpu": "1", "memory": "256Mi"},
        }}]}})
    );

    let only_requests = resize_patch(&ResizeSpec {
        container: "app".into(),
        requests: vec![("memory".into(), "64Mi".into())],
        limits: vec![],
    });
    assert_eq!(
        only_requests.body["spec"]["containers"][0]["resources"],
        json!({"requests": {"memory": "64Mi"}})
    );
}
