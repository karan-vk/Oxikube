use oxikube_domain::access::{AccessCheck, AccessDecision, Impersonate};
use oxikube_domain::ids::Gvr;
use oxikube_ports::{ContextScope, Mention};
use serde_json::json;

use super::{Harness, cluster, object, role_binding};

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

fn pods(verb: &str, sa: &Impersonate) -> AccessCheck {
    AccessCheck::new(verb, Gvr::new("", "", "pods"))
        .namespace("shop")
        .impersonating(sa.clone())
}

#[tokio::test]
async fn verb_rows_list_allowed_denied_and_unknown_verbs_apart() {
    let h = Harness::connected().await;
    let sa = Impersonate::service_account("shop", "bot");
    h.access()
        .set_decision(pods("list", &sa), AccessDecision::Denied { reason: None });
    h.access().set_decision(
        pods("create", &sa),
        AccessDecision::Unknown {
            reason: "evaluation error".into(),
        },
    );
    h.access().set_decision(
        AccessCheck::new("delete", Gvr::new("apps", "", "statefulsets"))
            .namespace("shop")
            .impersonating(sa.clone()),
        AccessDecision::Denied { reason: None },
    );
    h.access().set_decision(
        AccessCheck::new("*", Gvr::new("*", "", "*")).impersonating(sa),
        AccessDecision::Denied { reason: None },
    );
    let body = resolve(&h, "@rbac/sa/shop/bot", &scope())
        .await
        .expect("block");
    assert!(!body.contains("* on *"), "{body}");
    assert!(
        body.contains("pods: get patch delete (unknown: create)"),
        "{body}"
    );
    assert!(
        body.contains("statefulsets.apps: get list create patch\n"),
        "{body}"
    );
    assert!(
        body.contains("deployments.apps: get list create patch delete"),
        "{body}"
    );
}

#[tokio::test]
async fn a_user_subject_naming_the_service_account_is_a_binding() {
    let h = Harness::connected().await;
    h.objects().insert(role_binding(
        "shop",
        "as-user",
        "editor",
        json!([{"kind": "User", "name": "system:serviceaccount:shop:bot"}]),
    ));
    let body = resolve(&h, "@rbac/sa/shop/bot", &scope())
        .await
        .expect("block");
    assert!(body.contains("as-user -> Role editor"), "{body}");
}

/// Every ServiceAccount is in `system:authenticated`: a binding to that group applies to it, and
/// the block lists it beside the verbs it explains.
#[tokio::test]
async fn a_binding_to_system_authenticated_is_listed() {
    let h = Harness::connected().await;
    h.objects().insert(object(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRoleBinding",
        "metadata": {"name": "everyone-views"},
        "roleRef": {"kind": "ClusterRole", "name": "view", "apiGroup": "rbac.authorization.k8s.io"},
        "subjects": [{"kind": "Group", "name": "system:authenticated"}],
    })));
    h.objects().insert(object(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRoleBinding",
        "metadata": {"name": "other-group"},
        "roleRef": {"kind": "ClusterRole", "name": "edit", "apiGroup": "rbac.authorization.k8s.io"},
        "subjects": [{"kind": "Group", "name": "devs"}],
    })));
    let body = resolve(&h, "@rbac/sa/shop/builder", &scope())
        .await
        .expect("block");
    assert!(
        body.contains(
            "ClusterRoleBinding everyone-views -> ClusterRole view (via group system:authenticated)"
        ),
        "{body}"
    );
    assert!(!body.contains("other-group"), "{body}");
    assert!(!body.contains("no binding names"), "{body}");
}

/// When the first review already answers for everything (refused, or cluster-admin), the other
/// sixty are not sent.
#[tokio::test]
async fn a_refused_or_cluster_admin_first_review_sends_no_more() {
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
    assert_eq!(h.access().can_i_calls(), 1, "one review, then stop");

    let admin = Harness::connected().await;
    let body = resolve(&admin, "@rbac/sa/shop/bot", &scope())
        .await
        .expect("block");
    assert!(
        body.contains("* on *: allowed (cluster-admin level)"),
        "{body}"
    );
    assert!(
        body.contains("pods: get list create patch delete"),
        "{body}"
    );
    assert_eq!(
        admin.access().can_i_calls(),
        1,
        "everything is allowed already"
    );
}

/// A review that fails part-way stops the rest and says the summary is unavailable.
#[tokio::test]
async fn an_error_part_way_stops_the_remaining_reviews() {
    let h = Harness::connected().await;
    let access = h.access();
    let script = &access.script().can_i;
    // `*` on `*` is denied, the first resource review fails.
    script.push_ok(AccessDecision::Denied { reason: None });
    script.push_err(oxikube_domain::OxiError::network("connection reset"));
    let body = resolve(&h, "@rbac/sa/shop/bot", &scope())
        .await
        .expect("block");
    assert!(body.contains("(unavailable: connection reset)"), "{body}");
    assert!(
        h.access().can_i_calls() < 1 + 60,
        "stopped early: {} reviews",
        h.access().can_i_calls()
    );
}
