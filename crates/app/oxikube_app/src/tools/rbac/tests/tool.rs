use oxikube_domain::ErrorKind;
use oxikube_domain::access::{AccessCheck, AccessDecision, Impersonate};
use oxikube_domain::ids::Gvr;
use serde_json::json;

use super::{Harness, structured, text};

fn pods(verb: &str) -> AccessCheck {
    AccessCheck::new(verb, Gvr::new("", "v1", "pods")).namespace("shop")
}

#[tokio::test]
async fn the_tool_is_registered_read_only() {
    let h = Harness::new();
    let tool = h.tools.get("k8s.can_i").expect("registered");
    assert!(tool.def().risk.is_none(), "a read has no risk tier");
}

#[tokio::test]
async fn allowed_denied_and_unknown_are_three_answers() {
    let h = Harness::connected().await;
    let access = h.access();
    access.set_decision(pods("get"), AccessDecision::Allowed);
    access.set_decision(
        pods("delete"),
        AccessDecision::Denied {
            reason: Some("no rule".into()),
        },
    );
    access.set_decision(
        pods("patch"),
        AccessDecision::Unknown {
            reason: "webhook down".into(),
        },
    );
    let call = |verb: &'static str| {
        h.can_i(json!({"verb": verb, "resource": "pods", "namespace": "shop"}))
    };
    let allowed = call("get").await.expect("ok");
    assert_eq!(structured(&allowed)["allowed"], true);
    assert_eq!(structured(&allowed)["method"], "SelfSubjectAccessReview");
    let denied = call("delete").await.expect("ok");
    assert_eq!(structured(&denied)["decision"], "denied");
    assert_eq!(structured(&denied)["reason"], "no rule");
    let unknown = call("patch").await.expect("ok");
    assert_eq!(structured(&unknown)["decision"], "unknown");
    assert_eq!(
        structured(&unknown)["reason"],
        "webhook down",
        "evaluationError propagates"
    );
    assert_eq!(structured(&unknown)["allowed"], false);
    assert!(text(&denied).contains("kubectl auth can-i delete pods -n shop"));
}

#[tokio::test]
async fn an_impersonated_subject_asks_a_subject_review() {
    let h = Harness::connected().await;
    let sa = Impersonate::service_account("shop", "bot");
    h.access().set_decision(
        pods("delete").impersonating(sa),
        AccessDecision::Denied { reason: None },
    );
    let out = h
        .can_i(json!({
            "verb": "delete", "resource": "pods", "namespace": "shop",
            "subject": {"kind": "serviceaccount", "name": "bot", "namespace": "shop"}
        }))
        .await
        .expect("ok");
    let result = structured(&out);
    assert_eq!(result["decision"], "denied");
    assert_eq!(result["method"], "SubjectAccessReview");
    assert_eq!(result["reason"], "no RBAC rule allows it");
    assert!(
        result["kubectl"]
            .as_str()
            .expect("kubectl")
            .contains("--as system:serviceaccount:shop:bot")
    );
    assert_eq!(h.access().can_i_calls(), 1, "the cache is not consulted");
}

#[tokio::test]
async fn a_malformed_call_is_an_error_and_an_unreachable_cluster_a_tool_error() {
    let h = Harness::new();
    let missing = h
        .can_i(json!({"resource": "pods"}))
        .await
        .expect_err("schema");
    assert_eq!(missing.kind(), ErrorKind::Validation);
    let extra = h
        .can_i(json!({"verb": "get", "resource": "pods", "as": "x"}))
        .await
        .expect_err("schema");
    assert_eq!(extra.kind(), ErrorKind::Validation);
    let out = h
        .can_i(json!({"verb": "get", "resource": "pods"}))
        .await
        .expect("ok");
    assert!(out.is_error, "no connected cluster is told to the model");
}

#[tokio::test]
async fn the_service_names_the_method() {
    let h = Harness::connected().await;
    let req = crate::tools::rbac::CanIRequest {
        verb: "get".into(),
        resource: "pods".into(),
        ..Default::default()
    };
    let answer = h
        .service
        .ask(&super::cluster(), &req)
        .await
        .expect("answer");
    assert_eq!(answer.outcome(), "Allowed");
    assert!(
        answer
            .to_text()
            .contains("answered by: SelfSubjectAccessReview")
    );
}
