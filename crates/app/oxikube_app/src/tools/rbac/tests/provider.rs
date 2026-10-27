use oxikube_domain::access::{AccessCheck, AccessDecision, Impersonate};
use oxikube_domain::ids::Gvr;
use oxikube_ports::{ContextScope, Mention};
use serde_json::json;

use super::{Harness, cluster, role_binding};

fn scope() -> ContextScope {
    ContextScope::new().with_cluster(cluster())
}

async fn resolve(
    h: &Harness,
    mention: &str,
    scope: &ContextScope,
) -> oxikube_domain::OxiResult<String> {
    let blocks = h
        .contexts
        .resolve(&Mention::parse(mention).expect("mention"), scope)
        .await?;
    Ok(blocks[0].body.clone())
}

#[tokio::test]
async fn bindings_and_verbs_are_quoted_and_labelled() {
    let h = Harness::connected().await;
    h.objects().insert(role_binding(
        "shop",
        "bot-edit",
        "editor",
        json!([{"kind": "ServiceAccount", "name": "bot", "namespace": "shop"}]),
    ));
    h.objects().insert(role_binding(
        "shop",
        "other",
        "viewer",
        json!([{"kind": "ServiceAccount", "name": "someone-else"}]),
    ));
    let sa = Impersonate::service_account("shop", "bot");
    h.access().set_decision(
        AccessCheck::new("get", Gvr::new("", "", "pods"))
            .namespace("shop")
            .impersonating(sa),
        AccessDecision::Allowed,
    );
    let body = resolve(&h, "@rbac/sa/shop/bot", &scope())
        .await
        .expect("block");
    assert!(body.contains("# subject: ServiceAccount shop/bot"));
    assert!(body.contains("# bindings: from RoleBindings"));
    assert!(body.contains("bot-edit -> Role editor"));
    assert!(!body.contains("viewer"));
    assert!(body.contains("# verbs: SubjectAccessReview"));
}

#[tokio::test]
async fn the_verb_part_says_when_reviews_are_not_permitted() {
    let h = Harness::connected().await;
    h.access()
        .script()
        .can_i
        .push_err(oxikube_domain::OxiError::forbidden(
            "needs subjectaccessreviews",
        ));
    let body = resolve(&h, "@rbac/sa/shop/bot", &scope())
        .await
        .expect("block");
    assert!(body.contains("(unavailable: needs subjectaccessreviews)"));
    assert!(body.contains("no binding names this ServiceAccount"));
}

#[tokio::test]
async fn the_block_is_cut_to_the_budget_with_a_note() {
    let h = Harness::connected().await;
    let small = scope().with_max_total_bytes(400);
    let blocks = h
        .contexts
        .resolve(
            &Mention::parse("@rbac/sa/shop/bot").expect("mention"),
            &small,
        )
        .await
        .expect("blocks");
    assert!(blocks[0].body.len() <= 400);
    assert!(blocks[0].truncated);
    assert!(blocks[0].body.contains("left out to fit the size budget"));
}

#[tokio::test]
async fn a_mention_of_the_wrong_shape_is_a_validation_error() {
    let h = Harness::connected().await;
    assert!(resolve(&h, "@rbac/user/jane", &scope()).await.is_err());
    assert!(resolve(&h, "@rbac/sa/shop", &scope()).await.is_err());
}
