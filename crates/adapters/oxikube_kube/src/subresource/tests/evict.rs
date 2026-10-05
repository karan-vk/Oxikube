//! `evict`: the `Eviction` body, dry run, and a budget refusal mapped to a retryable error.

use http::Method;
use oxikube_domain::ErrorKind;
use oxikube_ports::{DeleteOptions, Preconditions, PropagationPolicy, ResourceWriter};
use serde_json::json;

use super::harness::*;
use crate::fake_api::status_body;
use crate::subresource::eviction_blocked;

fn success() -> serde_json::Value {
    json!({"kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Success", "code": 201})
}

fn pdb_refusal() -> serde_json::Value {
    json!({
        "kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Failure", "code": 429,
        "reason": "TooManyRequests",
        "message": "Cannot evict pod as it would violate the pod's disruption budget.",
        "details": {"causes": [{
            "reason": "DisruptionBudget",
            "message": "The disruption budget web needs 2 healthy pods and has 2 currently",
        }]},
    })
}

#[tokio::test]
async fn evict_posts_a_policy_v1_eviction() {
    let api = server();
    api.reply(POD_EVICTION, 201, success());
    resources(&api)
        .evict("default", "p", &DeleteOptions::default())
        .await
        .expect("evict");
    let sent = only(&api, POD_EVICTION);
    assert_eq!(sent.method, Method::POST);
    assert_eq!(sent.content_type.as_deref(), Some("application/json"));
    assert_eq!(
        sent.body,
        Some(json!({
            "apiVersion": "policy/v1", "kind": "Eviction",
            "metadata": {"name": "p", "namespace": "default"},
            "deleteOptions": {},
        }))
    );
    assert_eq!(query(&sent), Vec::<String>::new());
}

#[tokio::test]
async fn delete_options_reach_the_server_in_the_field_it_reads() {
    let api = server();
    api.reply(POD_EVICTION, 201, success());
    let options = DeleteOptions {
        dry_run: true,
        grace_period_secs: Some(5),
        propagation: Some(PropagationPolicy::Foreground),
        preconditions: Some(Preconditions {
            resource_version: Some("12".into()),
            uid: Some("uid-1".into()),
        }),
    };
    resources(&api)
        .evict("default", "p", &options)
        .await
        .expect("dry-run evict");
    let sent = only(&api, POD_EVICTION);
    assert_eq!(query(&sent), ["dryRun=All"]);
    assert_eq!(
        sent.body.expect("body")["deleteOptions"],
        json!({
            "dryRun": ["All"], "gracePeriodSeconds": 5, "propagationPolicy": "Foreground",
            "preconditions": {"resourceVersion": "12", "uid": "uid-1"},
        })
    );
}

#[tokio::test]
async fn a_budget_refusal_is_retryable_and_names_the_budget() {
    let api = server();
    api.reply(POD_EVICTION, 429, pdb_refusal());
    let err = resources(&api)
        .evict("default", "p", &DeleteOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Network);
    assert!(err.is_retryable());
    let blocked = eviction_blocked(&err).expect("blocked marker");
    assert_eq!(
        blocked.reason,
        "The disruption budget web needs 2 healthy pods and has 2 currently"
    );
    assert!(err.message().contains("default/p"), "{err}");
    assert!(err.message().contains("disruption budget web"), "{err}");
}

#[tokio::test]
async fn a_throttled_429_is_retryable_but_not_a_budget_refusal() {
    let api = server();
    api.reply(
        POD_EVICTION,
        429,
        status_body(
            429,
            "TooManyRequests",
            "too many requests, please try again later",
        ),
    );
    let err = resources(&api)
        .evict("default", "p", &DeleteOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Network);
    assert!(err.is_retryable());
    assert!(eviction_blocked(&err).is_none());
}

#[tokio::test]
async fn other_failures_keep_their_kind_and_carry_no_marker() {
    let cases = [
        (
            404,
            json!({"kind": "Status", "apiVersion": "v1", "status": "Failure", "code": 404,
                   "reason": "NotFound", "message": "pods \"p\" not found",
                   "details": {"name": "p", "kind": "pods"}}),
            ErrorKind::NotFound,
        ),
        (
            403,
            status_body(403, "Forbidden", "pods/eviction is forbidden"),
            ErrorKind::Forbidden,
        ),
        (
            500,
            status_body(
                500,
                "InternalError",
                "This pod has more than one PodDisruptionBudget",
            ),
            ErrorKind::Internal,
        ),
    ];
    for (code, body, kind) in cases {
        let api = server();
        api.reply(POD_EVICTION, code, body);
        let err = resources(&api)
            .evict("default", "p", &DeleteOptions::default())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), kind, "{code}: {err}");
        assert!(eviction_blocked(&err).is_none(), "{code}");
    }
}

#[tokio::test]
async fn evict_validates_its_names_before_any_request() {
    let api = server();
    let r = resources(&api);
    for (namespace, pod) in [
        ("", "p"),
        ("default", ""),
        ("a/b", "p"),
        ("default", "../x"),
    ] {
        let err = r
            .evict(namespace, pod, &DeleteOptions::default())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation, "{namespace:?} {pod:?}");
    }
    assert!(calls(&api).is_empty());
}

#[tokio::test]
async fn evictions_use_the_unretried_client_when_one_is_set() {
    let shared = server();
    let unretried = server();
    unretried.reply(POD_EVICTION, 429, pdb_refusal());
    let r = resources(&shared).with_unretried_client(unretried.client());
    let err = r
        .evict("default", "p", &DeleteOptions::default())
        .await
        .unwrap_err();
    assert!(eviction_blocked(&err).is_some(), "{err}");
    assert_eq!(unretried.hits(POD_EVICTION), 1);
    assert_eq!(shared.hits(POD_EVICTION), 0);
}
