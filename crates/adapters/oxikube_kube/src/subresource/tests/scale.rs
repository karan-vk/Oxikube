//! `get_scale` and `scale`: the scale subresource, its request and its errors.

use http::Method;
use oxikube_domain::ErrorKind;
use oxikube_ports::{ResourceReader, ResourceWriter, Scale, WriteOptions};
use serde_json::json;

use super::harness::*;
use crate::fake_api::status_body;

#[tokio::test]
async fn get_scale_reads_the_subresource_into_the_port_type() {
    let api = server();
    api.reply(DEPLOY_SCALE, 200, scale_body(3, 2, "77"));
    let scale = resources(&api)
        .get_scale(&deployment_gvk(), Some("default"), "web")
        .await
        .expect("get_scale");
    assert_eq!(
        scale,
        Scale {
            replicas: 3,
            current_replicas: 2,
            selector: Some("app=web".into()),
            resource_version: Some("77".into()),
        }
    );
    let sent = only(&api, DEPLOY_SCALE);
    assert_eq!(sent.method, Method::GET);
    assert_eq!(sent.body, None);
}

#[tokio::test]
async fn a_scale_without_counts_reads_as_zero() {
    let api = server();
    api.reply(
        DEPLOY_SCALE,
        200,
        json!({"metadata": {"name": "web"}, "spec": {}, "status": {}}),
    );
    let scale = resources(&api)
        .get_scale(&deployment_gvk(), Some("default"), "web")
        .await
        .expect("get_scale");
    assert_eq!(scale, Scale::default());
}

#[tokio::test]
async fn scale_patches_the_scale_subresource_not_the_object() {
    let api = server();
    api.reply(DEPLOY_SCALE, 200, scale_body(5, 2, "78"));
    let scale = resources(&api)
        .scale(
            &deployment_gvk(),
            Some("default"),
            "web",
            5,
            &WriteOptions::default(),
        )
        .await
        .expect("scale");
    assert_eq!(scale.replicas, 5);
    assert_eq!(scale.current_replicas, 2);

    let sent = only(&api, DEPLOY_SCALE);
    assert_eq!(sent.method, Method::PATCH);
    assert_eq!(
        sent.content_type.as_deref(),
        Some("application/merge-patch+json")
    );
    assert_eq!(sent.body, Some(json!({"spec": {"replicas": 5}})));
    assert_eq!(query(&sent), ["fieldManager=oxikube"]);
    // Nothing touched `/deployments/web` itself.
    assert!(calls(&api).iter().all(|r| r.path == DEPLOY_SCALE));
}

#[tokio::test]
async fn scale_to_zero_and_dry_run_are_passed_through() {
    let api = server();
    api.reply(DEPLOY_SCALE, 200, scale_body(0, 2, "79"));
    let options = WriteOptions {
        field_manager: Some("tester".into()),
        ..WriteOptions::dry_run()
    };
    let scale = resources(&api)
        .scale(&deployment_gvk(), Some("default"), "web", 0, &options)
        .await
        .expect("scale to zero");
    assert_eq!(scale.replicas, 0);
    let sent = only(&api, DEPLOY_SCALE);
    assert_eq!(sent.body, Some(json!({"spec": {"replicas": 0}})));
    assert_eq!(query(&sent), ["dryRun=All", "fieldManager=tester"]);
}

#[tokio::test]
async fn negative_replicas_are_rejected_before_any_request() {
    let api = server();
    let err = resources(&api)
        .scale(
            &deployment_gvk(),
            Some("default"),
            "web",
            -1,
            &WriteOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(calls(&api).is_empty());
}

#[tokio::test]
async fn a_kind_without_a_scale_subresource_is_unsupported() {
    let api = server();
    // The object exists; the server does not know the path (a CRD names the object in its 404).
    api.reply(
        WIDGET_SCALE,
        404,
        json!({"kind": "Status", "apiVersion": "v1", "status": "Failure", "code": 404,
               "reason": "NotFound", "message": "widgets.test.oxikube.dev \"w\" not found",
               "details": {"name": "w", "kind": "widgets"}}),
    );
    api.reply(WIDGET, 200, json!({"metadata": {"name": "w"}}));
    let r = resources(&api);
    let err = r
        .get_scale(&widget_gvk(), Some("default"), "w")
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported, "{err}");
    assert!(err.message().contains("scale"), "{err}");

    let err = r
        .scale(
            &widget_gvk(),
            Some("default"),
            "w",
            2,
            &WriteOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported, "{err}");
}

#[tokio::test]
async fn scaling_a_missing_object_is_not_found() {
    let api = server();
    api.reply(
        DEPLOY_SCALE,
        404,
        json!({"kind": "Status", "apiVersion": "v1", "status": "Failure", "code": 404,
               "reason": "NotFound", "message": "deployments.apps \"web\" not found",
               "details": {"name": "web", "group": "apps", "kind": "deployments"}}),
    );
    let err = resources(&api)
        .scale(
            &deployment_gvk(),
            Some("default"),
            "web",
            1,
            &WriteOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound, "{err}");
    // The object was probed to tell the two 404s apart; it is missing too (unscripted: 404).
    assert_eq!(api.hits(DEPLOY), 1);
}

#[tokio::test]
async fn a_forbidden_scale_is_forbidden() {
    let api = server();
    api.reply(
        DEPLOY_SCALE,
        403,
        status_body(
            403,
            "Forbidden",
            "deployments.apps/scale \"web\" is forbidden",
        ),
    );
    let err = resources(&api)
        .scale(
            &deployment_gvk(),
            Some("default"),
            "web",
            1,
            &WriteOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Forbidden);
}

#[tokio::test]
async fn a_scale_on_a_namespaced_kind_needs_a_namespace() {
    let api = server();
    let err = resources(&api)
        .get_scale(&deployment_gvk(), None, "web")
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
}
