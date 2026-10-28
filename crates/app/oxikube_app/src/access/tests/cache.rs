//! TTL, invalidation, single flight and failures of the rules cache.

use futures::future::join_all;
use oxikube_domain::OxiError;
use oxikube_domain::access::Access;

use super::*;

#[tokio::test]
async fn a_fresh_review_is_served_from_the_cache() {
    let (service, port, _clock) = service(FakeAccessReviewPort::new().with_rules(reads(&["pods"])));

    for _ in 0..5 {
        let rules = service.rules(Some("dev")).await.expect("rules");
        assert_eq!(rules.level("list", "", "pods"), Access::Granted);
    }

    assert_eq!(port.rules_calls(Some("dev")), 1);
}

#[tokio::test]
async fn each_namespace_has_its_own_entry() {
    let (service, port, _clock) = service(
        FakeAccessReviewPort::new()
            .with_namespace_rules("dev", reads(&["pods"]))
            .with_namespace_rules("prod", reads(&["secrets"])),
    );

    let dev = service.rules(Some("dev")).await.expect("dev");
    let prod = service.rules(Some("prod")).await.expect("prod");

    assert_eq!(dev.level("list", "", "pods"), Access::Granted);
    assert_eq!(prod.level("list", "", "pods"), Access::Denied);
    assert_eq!(port.rules_calls(Some("dev")), 1);
    assert_eq!(port.rules_calls(Some("prod")), 1);
}

#[tokio::test]
async fn the_review_expires_after_the_ttl() {
    let (service, port, clock) = service(FakeAccessReviewPort::new());
    service.rules(Some("dev")).await.expect("first");
    clock.advance(DEFAULT_TTL - Duration::from_secs(1));
    service.rules(Some("dev")).await.expect("still fresh");
    assert_eq!(port.rules_calls(Some("dev")), 1);
    assert!(service.peek_rules(Some("dev")).is_some());

    clock.advance(Duration::from_secs(2));
    assert!(service.peek_rules(Some("dev")).is_none(), "expired");
    service.rules(Some("dev")).await.expect("refetched");

    assert_eq!(port.rules_calls(Some("dev")), 2);
}

#[tokio::test]
async fn invalidating_a_namespace_forgets_only_it() {
    let (service, port, _clock) = service(FakeAccessReviewPort::new());
    service.rules(Some("dev")).await.expect("dev");
    service.rules(Some("prod")).await.expect("prod");

    service.invalidate_namespace(Some("dev"));
    service.rules(Some("dev")).await.expect("dev again");
    service.rules(Some("prod")).await.expect("prod cached");

    assert_eq!(port.rules_calls(Some("dev")), 2);
    assert_eq!(port.rules_calls(Some("prod")), 1);
}

#[tokio::test]
async fn invalidating_everything_forgets_every_namespace() {
    let (service, port, _clock) = service(FakeAccessReviewPort::new());
    service.rules(Some("dev")).await.expect("dev");
    service.rules(None).await.expect("cluster-wide");

    service.invalidate();

    assert!(service.peek_rules(Some("dev")).is_none());
    assert!(service.peek_rules(None).is_none());
    service.rules(Some("dev")).await.expect("dev again");
    assert_eq!(port.rules_calls(Some("dev")), 2);
}

#[tokio::test]
async fn invalidation_reaches_the_adapter_cache_too() {
    let (service, port, _clock) = service(FakeAccessReviewPort::new());
    service.rules(Some("dev")).await.expect("dev");

    service.invalidate_namespace(Some("dev"));
    service.invalidate_namespace(None);
    service.invalidate();

    assert_eq!(port.rules_invalidations(Some("dev")), 1);
    assert_eq!(port.rules_invalidations(None), 1);
    assert_eq!(port.full_rules_invalidations(), 1);
}

#[tokio::test]
async fn ten_concurrent_asks_make_one_review() {
    let (service, port, _clock) = service(FakeAccessReviewPort::new().with_rules_yields(3));

    let asks = (0..10).map(|_| service.rules(Some("dev")));
    let answers = join_all(asks).await;

    assert!(answers.iter().all(Result::is_ok));
    assert_eq!(port.rules_calls(Some("dev")), 1, "single flight");
    let first = answers[0].as_ref().expect("ok");
    assert!(
        answers
            .iter()
            .all(|a| Arc::ptr_eq(a.as_ref().expect("ok"), first))
    );
}

#[tokio::test]
async fn an_invalidation_during_a_review_discards_its_stale_answer() {
    let (service, port, _clock) = service(FakeAccessReviewPort::new().with_rules_yields(2));
    let slow = service.rules(Some("dev"));
    let invalidate = async {
        service.invalidate();
    };

    let (answer, ()) = futures::join!(slow, invalidate);

    assert!(answer.is_ok(), "the asker still gets its answer");
    assert!(
        service.peek_rules(Some("dev")).is_none(),
        "but it is not cached"
    );
    assert_eq!(port.rules_calls(Some("dev")), 1);
}

#[tokio::test]
async fn a_failed_review_is_an_error_and_is_not_cached() {
    let (service, port, _clock) = service(FakeAccessReviewPort::new());
    port.script()
        .rules
        .push_err(OxiError::network("apiserver unreachable"));

    let failed = service.rules(Some("dev")).await.expect_err("fails");
    assert_eq!(failed.kind(), oxikube_domain::ErrorKind::Network);
    assert!(failed.is_retryable());
    service.rules(Some("dev")).await.expect("retried");

    assert_eq!(port.rules_calls(Some("dev")), 2);
}

#[tokio::test]
async fn the_cluster_wide_ask_is_marked_namespaces_unseen() {
    let (service, _port, _clock) =
        service(FakeAccessReviewPort::new().with_rules(reads(&["pods"])));

    let cluster_wide = service.rules(None).await.expect("cluster-wide");
    let namespaced = service.rules(Some("dev")).await.expect("dev");

    assert!(cluster_wide.namespaces_unseen);
    assert!(!namespaced.namespaces_unseen);
    // A namespaced resource the probe namespace did not grant is unknown, not denied.
    assert_eq!(cluster_wide.level("list", "", "secrets"), Access::Unknown);
}
