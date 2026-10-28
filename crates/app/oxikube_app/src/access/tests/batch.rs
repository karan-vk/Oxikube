//! `peek`, `can_i` and the batch `can_i_many`.

use oxikube_domain::OxiError;
use oxikube_domain::access::{AccessDecision, AccessRule, AccessRules};

use super::*;

fn denied(decision: &AccessDecision) -> bool {
    matches!(decision, AccessDecision::Denied { .. })
}

#[tokio::test]
async fn peek_answers_only_from_a_fresh_cached_review() {
    let (service, port, clock) = service(FakeAccessReviewPort::new().with_rules(reads(&["pods"])));
    assert_eq!(
        service.peek(&check("list", "dev")),
        None,
        "not reviewed yet"
    );

    service.rules(Some("dev")).await.expect("rules");

    assert_eq!(
        service.peek(&check("list", "dev")),
        Some(AccessDecision::Allowed)
    );
    assert!(denied(
        &service.peek(&check("delete", "dev")).expect("decided")
    ));
    assert_eq!(
        service.peek(&check("list", "prod")),
        None,
        "other namespace"
    );
    clock.advance(PAST_TTL);
    assert_eq!(service.peek(&check("list", "dev")), None, "expired");
    assert_eq!(port.can_i_calls(), 0, "peek never asks");
}

#[tokio::test]
async fn a_batch_is_answered_from_one_review_per_namespace() {
    let (service, port, _clock) = service(
        FakeAccessReviewPort::new()
            .with_namespace_rules("dev", reads(&["pods"]))
            .with_namespace_rules("prod", AccessRules::none()),
    );
    let checks = [
        check("list", "dev"),
        check("get", "dev"),
        check("delete", "dev"),
        check("list", "prod"),
    ];

    let decisions = service.can_i_many(&checks).await;

    assert_eq!(decisions[0], AccessDecision::Allowed);
    assert_eq!(decisions[1], AccessDecision::Allowed);
    assert!(denied(&decisions[2]));
    assert!(denied(&decisions[3]));
    assert_eq!(port.rules_calls(Some("dev")), 1);
    assert_eq!(port.rules_calls(Some("prod")), 1);
    assert_eq!(port.can_i_calls(), 0, "the rules could decide everything");
}

#[tokio::test]
async fn a_second_batch_makes_no_network_call() {
    let (service, port, _clock) = service(FakeAccessReviewPort::new().with_rules(reads(&["pods"])));
    let checks: Vec<_> = (0..60)
        .map(|i| check("list", &format!("ns-{}", i % 3)))
        .collect();
    service.can_i_many(&checks).await;
    let calls = port.recorded_calls().len();

    let again = service.can_i_many(&checks).await;

    assert_eq!(
        port.recorded_calls().len(),
        calls,
        "zero calls after the first review"
    );
    assert!(again.iter().all(|d| *d == AccessDecision::Allowed));
}

#[tokio::test]
async fn only_what_the_rules_cannot_decide_asks_the_server() {
    let named =
        AccessRules::none().with_rule(AccessRule::granting(&["get"], &[""], &["pods"], &["web"]));
    let web = check("get", "dev").name("web");
    let db = check("get", "dev").name("db");
    let (service, port, _clock) = service(
        FakeAccessReviewPort::new()
            .with_namespace_rules("dev", named)
            .with_decision(web.clone(), AccessDecision::Allowed)
            .with_decision(db.clone(), AccessDecision::Denied { reason: None }),
    );

    let decisions = service.can_i_many(&[web, db, check("delete", "dev")]).await;

    assert_eq!(decisions[0], AccessDecision::Allowed);
    assert!(denied(&decisions[1]));
    assert!(denied(&decisions[2]), "decided by the rules");
    assert_eq!(port.can_i_calls(), 2);
}

#[tokio::test]
async fn a_partial_review_is_unknown_to_the_rules_so_the_server_decides() {
    let (service, port, _clock) = service(
        FakeAccessReviewPort::new()
            .with_rules(reads(&["pods"]).into_partial())
            .with_decision(check("get", "dev"), AccessDecision::Allowed),
    );
    let secrets = AccessCheck::new("get", Gvr::new("", "v1", "secrets")).namespace("dev");

    let decisions = service
        .can_i_many(&[check("get", "dev"), secrets.clone()])
        .await;

    assert_eq!(decisions[0], AccessDecision::Allowed);
    assert_eq!(
        decisions[1],
        AccessDecision::Allowed,
        "fake default; the server was asked"
    );
    assert_eq!(port.can_i_calls(), 1, "only the unmatched secrets check");
}

#[tokio::test]
async fn an_incomplete_answer_is_unknown_never_denied() {
    let (service, port, _clock) =
        service(FakeAccessReviewPort::new().with_rules(AccessRules::none().into_partial()));
    port.script().can_i.push_ok(AccessDecision::Unknown {
        reason: "webhook authorizer down".into(),
    });

    let decisions = service.can_i_many(&[check("get", "dev")]).await;

    assert!(matches!(decisions[0], AccessDecision::Unknown { .. }));
    assert!(decisions[0].is_offered(), "a soft gate keeps the action");
}

#[tokio::test]
async fn failed_reviews_fail_open_as_unknown() {
    let (service, port, _clock) = service(FakeAccessReviewPort::new());
    // The batch's prefetch and the per-check fallback both fail.
    port.script().rules.push_err(OxiError::network("down"));
    port.script().rules.push_err(OxiError::network("down"));
    port.script().can_i.push_err(OxiError::network("down"));

    let decisions = service.can_i_many(&[check("get", "dev")]).await;

    assert!(matches!(decisions[0], AccessDecision::Unknown { .. }));
    let direct = service.can_i(&check("get", "dev")).await;
    assert!(direct.is_ok(), "the next ask works again: {direct:?}");
}

#[tokio::test]
async fn impersonated_checks_skip_the_callers_rules() {
    use oxikube_domain::access::Impersonate;
    let (service, port, _clock) = service(FakeAccessReviewPort::new());
    let as_bob = check("get", "dev").impersonating(Impersonate {
        user: "bob".into(),
        groups: Vec::new(),
    });

    let decision = service.can_i(&as_bob).await.expect("decision");

    assert_eq!(decision, AccessDecision::Allowed);
    assert_eq!(port.can_i_calls(), 1);
    assert_eq!(port.rules_calls(Some("dev")), 0);
}
