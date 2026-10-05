//! `KubePods` against the fake API: the requests it builds and how it reads the answers.

use std::time::Duration;

use http::Method;
use oxikube_domain::ErrorKind;
use serde_json::json;

use crate::fake_api::{FakeApi, status_body};
use crate::remote::exec::node_shell::{NODE_SHELL_LABEL, NodeShellConfig, node_shell_manifest};
use crate::remote::exec::pods::{KubePods, PodShape, Pods};
use crate::remote::exec::wait::Container;
use crate::subresource::{EphemeralContainerSpec, ephemeral_container_patch};

const PODS: &str = "/api/v1/namespaces/ns/pods";

#[tokio::test]
async fn create_posts_the_manifest_and_returns_the_generated_name() {
    let api = FakeApi::new();
    api.reply(
        PODS,
        201,
        json!({"metadata": {"name": "oxikube-node-shell-x7k2p", "namespace": "ns"}}),
    );
    let manifest = node_shell_manifest("n1", &NodeShellConfig::default()).expect("manifest");
    let name = KubePods::new(api.client())
        .create("ns", &manifest)
        .await
        .expect("create");
    assert_eq!(name, "oxikube-node-shell-x7k2p");
    let request = &api.requests()[0];
    assert_eq!(request.method, Method::POST);
    let body = request.body.as_ref().expect("body");
    assert_eq!(body["spec"]["nodeName"], "n1");
    assert_eq!(
        body["spec"]["containers"][0]["securityContext"]["privileged"],
        true
    );
}

#[tokio::test]
async fn create_refused_by_the_server_keeps_its_kind() {
    let api = FakeApi::new();
    api.reply(
        PODS,
        403,
        status_body(403, "Forbidden", "pods is forbidden"),
    );
    let manifest = node_shell_manifest("n1", &NodeShellConfig::default()).expect("manifest");
    let err = KubePods::new(api.client())
        .create("ns", &manifest)
        .await
        .expect_err("refused");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
}

#[tokio::test]
async fn delete_has_no_grace_period_and_a_missing_pod_is_fine() {
    let api = FakeApi::new();
    let path = format!("{PODS}/shell");
    api.reply(&path, 200, json!({"metadata": {"name": "shell"}}));
    let pods = KubePods::new(api.client());
    pods.delete("ns", "shell").await.expect("delete");
    let request = &api.requests()[0];
    assert_eq!(request.method, Method::DELETE);
    assert_eq!(
        request.body.as_ref().expect("options")["gracePeriodSeconds"],
        0
    );

    let gone = FakeApi::new();
    gone.reply(&path, 404, status_body(404, "NotFound", "gone"));
    KubePods::new(gone.client())
        .delete("ns", "shell")
        .await
        .expect("already gone");

    let denied = FakeApi::new();
    denied.reply(&path, 403, status_body(403, "Forbidden", "no"));
    let err = KubePods::new(denied.client())
        .delete("ns", "shell")
        .await
        .expect_err("denied");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
}

#[tokio::test]
async fn list_selects_by_label_and_reads_names_and_ages() {
    let api = FakeApi::new();
    api.reply(
        PODS,
        200,
        json!({"metadata": {}, "items": [
            {"metadata": {"name": "a", "creationTimestamp": "2026-01-01T00:00:00Z"}},
            {"metadata": {"name": "b"}},
        ]}),
    );
    let found = KubePods::new(api.client())
        .list("ns", NODE_SHELL_LABEL)
        .await
        .expect("list");
    assert!(
        api.requests()[0]
            .query
            .contains("labelSelector=oxikube.dev%2Fnode-shell")
    );
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].name, "a");
    assert_eq!(found[0].created, 1_767_225_600);
    assert_eq!(found[1].created, 0);
}

#[tokio::test]
async fn shape_lists_every_kind_of_container() {
    let api = FakeApi::new();
    api.reply(
        &format!("{PODS}/p"),
        200,
        json!({"metadata": {"name": "p"},
               "spec": {"containers": [{"name": "app", "image": "i"}],
                        "initContainers": [{"name": "init", "image": "i"}],
                        "ephemeralContainers": [{"name": "dbg", "image": "i"}]},
               "status": {"phase": "Running"}}),
    );
    let pods = KubePods::new(api.client());
    let shape = pods.shape("ns", "p").await.expect("shape");
    assert_eq!(
        shape,
        Some(PodShape {
            phase: "Running".into(),
            deleting: false,
            containers: vec!["app".into(), "init".into(), "dbg".into()],
            regular: vec!["app".into()],
        })
    );
    assert_eq!(pods.shape("ns", "missing").await.expect("lookup"), None);
}

#[tokio::test]
async fn the_ephemeral_container_patch_goes_to_its_subresource() {
    let api = FakeApi::new();
    api.reply(
        &format!("{PODS}/p/ephemeralcontainers"),
        200,
        json!({"metadata": {"name": "p"}}),
    );
    let patch = ephemeral_container_patch(&EphemeralContainerSpec {
        name: "dbg".into(),
        image: "busybox".into(),
        stdin: true,
        tty: true,
        ..EphemeralContainerSpec::default()
    });
    KubePods::new(api.client())
        .add_ephemeral_container("ns", "p", &patch.body)
        .await
        .expect("patch");
    let request = &api.requests()[0];
    assert_eq!(request.method, Method::PATCH);
    assert_eq!(
        request.content_type.as_deref(),
        Some("application/strategic-merge-patch+json")
    );
    assert_eq!(
        request.body.as_ref().expect("body")["spec"]["ephemeralContainers"][0]["name"],
        "dbg"
    );
}

fn pending_pod() -> serde_json::Value {
    json!({"metadata": {"name": "p", "namespace": "ns", "resourceVersion": "2"},
           "status": {"phase": "Pending"}})
}

#[tokio::test]
async fn waiting_for_a_pod_that_is_not_there_is_not_found_not_a_timeout() {
    let api = FakeApi::new();
    api.reply(
        PODS,
        200,
        json!({"metadata": {"resourceVersion": "1"}, "items": []}),
    );
    let started = std::time::Instant::now();
    let err = KubePods::new(api.client())
        .wait_running(
            "ns",
            "p",
            &Container::Regular("shell".into()),
            Duration::from_secs(30),
        )
        .await
        .expect_err("pod absent");
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "did not wait out the timeout"
    );
}

#[tokio::test]
async fn a_pod_deleted_while_waiting_is_not_found_not_a_timeout() {
    let api = FakeApi::new();
    api.reply(
        PODS,
        200,
        json!({"metadata": {"resourceVersion": "1"}, "items": [pending_pod()]}),
    );
    api.reply_watch(
        PODS,
        200,
        &[json!({"type": "DELETED", "object": pending_pod()})],
    );
    let started = std::time::Instant::now();
    let err = KubePods::new(api.client())
        .wait_running(
            "ns",
            "p",
            &Container::Regular("shell".into()),
            Duration::from_secs(30),
        )
        .await
        .expect_err("pod deleted");
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "did not wait out the timeout"
    );
}
