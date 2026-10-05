//! The generic subresource methods: status, ephemeral containers, resize, create and any
//! other name; request shape, scope rules and name validation.

use http::Method;
use oxikube_domain::ErrorKind;
use oxikube_ports::{Patch, PatchKind, ResourceReader, ResourceWriter, Subresource, WriteOptions};
use serde_json::json;

use super::harness::*;
use crate::fake_api::status_body;
use crate::subresource::{
    EphemeralContainerSpec, ResizeSpec, ephemeral_container_patch, resize_patch,
};

fn widget(rv: &str) -> serde_json::Value {
    json!({"apiVersion": "test.oxikube.dev/v1", "kind": "Widget",
           "metadata": {"name": "w", "namespace": "default", "resourceVersion": rv},
           "spec": {"size": 1}, "status": {"phase": "Ready"}})
}

#[tokio::test]
async fn get_subresource_returns_the_response_json_untouched() {
    let api = server();
    api.reply(WIDGET_STATUS, 200, widget("5"));
    let value = resources(&api)
        .get_subresource(&widget_gvk(), Some("default"), "w", &Subresource::Status)
        .await
        .expect("status");
    assert_eq!(value, widget("5"));
    assert_eq!(only(&api, WIDGET_STATUS).method, Method::GET);
}

#[tokio::test]
async fn patch_status_sends_the_patch_kind_and_options() {
    let api = server();
    api.reply(WIDGET_STATUS, 200, widget("6"));
    let patch = Patch::merge(json!({"status": {"phase": "Ready"}}));
    let value = resources(&api)
        .patch_subresource(
            &widget_gvk(),
            Some("default"),
            "w",
            &Subresource::Status,
            &patch,
            &WriteOptions::dry_run(),
        )
        .await
        .expect("patch status");
    assert_eq!(value["metadata"]["resourceVersion"], "6");
    let sent = only(&api, WIDGET_STATUS);
    assert_eq!(sent.method, Method::PATCH);
    assert_eq!(
        sent.content_type.as_deref(),
        Some("application/merge-patch+json")
    );
    assert_eq!(sent.body, Some(patch.body));
    assert_eq!(query(&sent), ["dryRun=All", "fieldManager=oxikube"]);
}

#[tokio::test]
async fn replace_status_puts_the_object() {
    let api = server();
    api.reply(WIDGET_STATUS, 200, widget("7"));
    resources(&api)
        .replace_subresource(
            &widget_gvk(),
            Some("default"),
            "w",
            &Subresource::Status,
            &widget("5"),
            &WriteOptions::default(),
        )
        .await
        .expect("replace status");
    let sent = only(&api, WIDGET_STATUS);
    assert_eq!(sent.method, Method::PUT);
    assert_eq!(sent.body, Some(widget("5")));
    assert_eq!(query(&sent), ["fieldManager=oxikube"]);
}

#[tokio::test]
async fn a_status_conflict_carries_conflict_details() {
    let api = server();
    api.reply(
        WIDGET_STATUS,
        409,
        status_body(
            409,
            "Conflict",
            "Operation cannot be fulfilled: the object has been modified",
        ),
    );
    let err = resources(&api)
        .replace_subresource(
            &widget_gvk(),
            Some("default"),
            "w",
            &Subresource::Status,
            &widget("1"),
            &WriteOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(err.conflict_details().is_some());
}

#[tokio::test]
async fn ephemeral_containers_go_to_their_subresource_as_a_strategic_patch() {
    let api = server();
    api.reply(
        POD_EPHEMERAL,
        200,
        json!({"kind": "Pod", "metadata": {"name": "p"}}),
    );
    let patch = ephemeral_container_patch(&EphemeralContainerSpec {
        name: "dbg".into(),
        image: "busybox".into(),
        target_container: Some("app".into()),
        ..EphemeralContainerSpec::default()
    });
    resources(&api)
        .patch_subresource(
            &pod_gvk(),
            Some("default"),
            "p",
            &Subresource::EphemeralContainers,
            &patch,
            &WriteOptions::default(),
        )
        .await
        .expect("ephemeral containers");
    let sent = only(&api, POD_EPHEMERAL);
    assert_eq!(sent.method, Method::PATCH);
    assert_eq!(
        sent.content_type.as_deref(),
        Some("application/strategic-merge-patch+json")
    );
    assert_eq!(sent.body, Some(patch.body));
}

#[tokio::test]
async fn resize_goes_to_the_resize_subresource() {
    let api = server();
    api.reply(
        POD_RESIZE,
        200,
        json!({"kind": "Pod", "metadata": {"name": "p"}}),
    );
    let patch = resize_patch(&ResizeSpec {
        container: "app".into(),
        requests: vec![("cpu".into(), "250m".into())],
        limits: vec![],
    });
    assert_eq!(patch.kind, PatchKind::Strategic);
    resources(&api)
        .patch_subresource(
            &pod_gvk(),
            Some("default"),
            "p",
            &Subresource::Resize,
            &patch,
            &WriteOptions::default(),
        )
        .await
        .expect("resize");
    assert_eq!(only(&api, POD_RESIZE).body, Some(patch.body));
}

#[tokio::test]
async fn a_cluster_scoped_kind_is_addressed_without_a_namespace() {
    let api = server();
    api.reply(
        NODE_STATUS,
        200,
        json!({"kind": "Node", "metadata": {"name": "n1"}}),
    );
    resources(&api)
        .get_subresource(&node_gvk(), None, "n1", &Subresource::Status)
        .await
        .expect("node status");
    let err = resources(&api)
        .get_subresource(&node_gvk(), Some("default"), "n1", &Subresource::Status)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[tokio::test]
async fn create_subresource_posts_the_body() {
    let api = server();
    let path = "/api/v1/namespaces/default/pods/p/binding";
    api.reply(path, 201, json!({"kind": "Status", "status": "Success"}));
    let body =
        json!({"apiVersion": "v1", "kind": "Binding", "target": {"kind": "Node", "name": "n1"}});
    resources(&api)
        .create_subresource(
            &pod_gvk(),
            Some("default"),
            "p",
            &Subresource::Other("binding".into()),
            &body,
            &WriteOptions::default(),
        )
        .await
        .expect("binding");
    let sent = only(&api, path);
    assert_eq!(sent.method, Method::POST);
    assert_eq!(sent.body, Some(body));
}

#[tokio::test]
async fn names_that_could_change_the_route_are_rejected_without_a_request() {
    let api = server();
    let r = resources(&api);
    for sub in ["", "a/b", "scale?x=1", "sta tus", "a#b", "%2e"] {
        let err = r
            .get_subresource(
                &pod_gvk(),
                Some("default"),
                "p",
                &Subresource::Other(sub.into()),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation, "{sub:?}");
    }
    for name in ["", "a/b", "p?x"] {
        let err = r
            .get_subresource(&pod_gvk(), Some("default"), name, &Subresource::Status)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation, "{name:?}");
    }
    assert!(calls(&api).is_empty());
}

#[tokio::test]
async fn a_kind_that_cannot_be_patched_is_unsupported_and_unknown_kinds_too() {
    let api = server();
    let r = resources(&api);
    let secret = oxikube_domain::ids::Gvk::new("", "v1", "Secret");
    let err = r
        .patch_subresource(
            &secret,
            Some("default"),
            "s",
            &Subresource::Status,
            &Patch::merge(json!({})),
            &WriteOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    let missing = oxikube_domain::ids::Gvk::new("nope.dev", "v1", "Nope");
    let err = r
        .get_subresource(&missing, Some("default"), "x", &Subresource::Status)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert!(calls(&api).is_empty());
}
