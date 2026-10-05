//! Sharing, idle teardown, reuse within the grace period and the limits, with fake feeds.

use std::time::Duration;

use oxikube_domain::ErrorKind;
use oxikube_ports::FeedVariant;

use super::*;

#[tokio::test(start_paused = true)]
async fn equal_requests_share_one_feed_and_count_subscribers() {
    let (registry, source) = registry(roomy());
    let mut first = registry.subscribe(full(pods(), "a")).await.unwrap();
    let mut second = registry.subscribe(full(pods(), "a")).await.unwrap();
    assert_eq!(source.opened(), vec![full(pods(), "a")]);
    assert!(first.take_feed().is_some(), "the opener carries the stream");
    assert!(second.take_feed().is_none(), "a joiner does not");

    let stats = registry.stats();
    assert_eq!(
        (stats.feeds, stats.subscribers, stats.idle_feeds),
        (1, 2, 0)
    );
    drop(first);
    assert_eq!(registry.stats().subscribers, 1);
    assert_eq!(registry.stats().idle_feeds, 0);
}

#[tokio::test(start_paused = true)]
async fn an_idle_feed_is_torn_down_after_the_grace_period() {
    let (registry, source) = registry(roomy());
    let mut lease = registry.subscribe(full(pods(), "a")).await.unwrap();
    let mut stream = lease.take_feed().unwrap().into_resources().unwrap();
    let fake = source.feed(&full(pods(), "a"));
    drop(lease);
    assert_eq!(registry.stats().idle_feeds, 1);

    tokio::time::sleep(Duration::from_secs(29)).await;
    assert!(fake.is_alive(), "still within the grace period");
    assert_eq!(registry.stats().feeds, 1);

    tokio::time::sleep(Duration::from_secs(2)).await;
    settle().await;
    assert!(
        !fake.is_alive(),
        "dropped (abort on drop) once the grace period ended"
    );
    assert!(
        next(&mut stream).await.is_none(),
        "the consumer's stream ends"
    );
    let stats = registry.stats();
    assert_eq!((stats.feeds, stats.started, stats.stopped), (0, 1, 1));
}

#[tokio::test(start_paused = true)]
async fn subscribing_again_within_the_grace_period_reuses_the_feed() {
    let (registry, source) = registry(roomy());
    let mut lease = registry.subscribe(full(pods(), "a")).await.unwrap();
    let mut stream = lease.take_feed().unwrap().into_resources().unwrap();
    let fake = source.feed(&full(pods(), "a"));
    drop(lease);

    tokio::time::sleep(Duration::from_secs(10)).await;
    let mut again = registry.subscribe(full(pods(), "a")).await.unwrap();
    assert_eq!(source.opened().len(), 1, "no second open");
    assert!(
        again.take_feed().is_none(),
        "the running stream stays with its consumer"
    );

    // The cancelled timer never fires: the feed outlives the old grace period.
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert!(fake.is_alive());
    fake.send(batch(vec![Delta::Applied(pod("a", "web", "2"))]));
    let item = next(&mut stream).await.unwrap().unwrap();
    assert_eq!(item.len(), 1);
    assert_eq!(registry.stats().idle_feeds, 0);

    // Idle again: a fresh grace period.
    drop(again);
    tokio::time::sleep(Duration::from_secs(31)).await;
    settle().await;
    assert!(!fake.is_alive());
}

#[tokio::test(start_paused = true)]
async fn a_zero_grace_period_tears_down_at_once() {
    let config = BudgetConfig {
        idle_grace: Duration::ZERO,
        ..roomy()
    };
    let (registry, source) = registry(config);
    let lease = registry.subscribe(full(pods(), "a")).await.unwrap();
    let fake = source.feed(&full(pods(), "a"));
    drop(lease);
    settle().await;
    assert!(!fake.is_alive());
    assert_eq!(registry.stats().feeds, 0);
}

#[tokio::test(start_paused = true)]
async fn dropping_the_stream_tears_the_feed_down_and_a_new_subscriber_reopens() {
    let (registry, source) = registry(roomy());
    let mut lease = registry.subscribe(full(pods(), "a")).await.unwrap();
    let fake = source.feed(&full(pods(), "a"));
    drop(lease.take_feed());
    settle().await;
    assert!(!fake.is_alive(), "no consumer, no feed");
    assert_eq!(registry.stats().feeds, 0);

    let mut again = registry.subscribe(full(pods(), "a")).await.unwrap();
    assert!(again.take_feed().is_some(), "a fresh feed with its stream");
    assert_eq!(source.opened().len(), 2);
    drop(lease);
    assert_eq!(
        registry.stats().subscribers,
        1,
        "the stale lease releases nothing"
    );
}

#[tokio::test(start_paused = true)]
async fn a_source_that_ends_delivers_its_last_item_and_leaves() {
    let (registry, source) = registry(roomy());
    let mut lease = registry.subscribe(full(pods(), "a")).await.unwrap();
    let mut stream = lease.take_feed().unwrap().into_resources().unwrap();
    let fake = source.feed(&full(pods(), "a"));
    fake.fail(OxiError::forbidden("role removed"));
    fake.end();
    let last = next(&mut stream).await.unwrap().unwrap_err();
    assert_eq!(last.kind(), ErrorKind::Forbidden);
    assert!(next(&mut stream).await.is_none());
    settle().await;
    let stats = registry.stats();
    assert_eq!((stats.feeds, stats.errors), (0, 1));
}

#[tokio::test(start_paused = true)]
async fn the_object_budget_degrades_to_metadata_then_refuses() {
    let config = BudgetConfig {
        metadata_above: 3,
        max_objects: 5,
        ..roomy()
    };
    let (registry, source) = registry(config);
    let mut lease = registry.subscribe(full(pods(), "a")).await.unwrap();
    let mut stream = lease.take_feed().unwrap().into_resources().unwrap();
    let three = (0..3).map(|i| pod("a", &format!("p{i}"), "1")).collect();
    source
        .feed(&full(pods(), "a"))
        .send(batch(vec![Delta::Restarted(three)]));
    next(&mut stream).await.unwrap().unwrap();
    assert!(!lease.is_degraded());

    // Three objects held: a new full feed opens metadata-only.
    let mut degraded = registry.subscribe(full(deployments(), "a")).await.unwrap();
    assert!(degraded.is_degraded());
    assert_eq!(degraded.variant(), FeedVariant::Metadata);
    let metadata = full(deployments(), "a").with_variant(FeedVariant::Metadata);
    assert_eq!(source.opened().last(), Some(&metadata));
    let mut deployments_stream = degraded.take_feed().unwrap().into_resources().unwrap();
    let two = (0..2).map(|i| pod("a", &format!("d{i}"), "1")).collect();
    source
        .feed(&metadata)
        .send(batch(vec![Delta::Restarted(two)]));
    next(&mut deployments_stream).await.unwrap().unwrap();

    // Five held, no idle feed to give back: refused, with the reason.
    let err = registry
        .subscribe(full(config_maps(), "a"))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::BudgetExceeded);
    assert!(!err.is_retryable());
    assert!(err.message().contains("5 objects (limit 5)"), "{err}");
    assert!(err.message().contains("ConfigMap"), "{err}");

    let stats = registry.stats();
    assert_eq!((stats.degraded, stats.refused, stats.objects), (1, 1, 5));
    assert_eq!(source.opened().len(), 2, "a refused feed is never opened");
}

#[tokio::test(start_paused = true)]
async fn a_degraded_request_joins_an_open_metadata_feed() {
    let config = BudgetConfig {
        metadata_above: 0,
        ..roomy()
    };
    let (registry, source) = registry(config);
    let first = registry.subscribe(full(pods(), "a")).await.unwrap();
    let second = registry.subscribe(full(pods(), "a")).await.unwrap();
    assert!(first.is_degraded() && second.is_degraded());
    assert_eq!(source.opened().len(), 1);
    assert_eq!(registry.stats().subscribers, 2);
}

#[tokio::test(start_paused = true)]
async fn the_feed_cap_evicts_idle_feeds_before_refusing() {
    let config = BudgetConfig {
        max_feeds: 2,
        ..roomy()
    };
    let (registry, source) = registry(config);
    let _a = registry.subscribe(full(pods(), "a")).await.unwrap();
    let b = registry.subscribe(full(pods(), "b")).await.unwrap();
    let fake_b = source.feed(&full(pods(), "b"));
    drop(b);

    // At the cap, but `b` is idle: it makes room.
    let _c = registry.subscribe(full(pods(), "c")).await.unwrap();
    settle().await;
    assert!(!fake_b.is_alive(), "the idle feed was evicted");
    let stats = registry.stats();
    assert_eq!((stats.feeds, stats.evicted), (2, 1));

    let err = registry.subscribe(full(pods(), "d")).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::BudgetExceeded);
    assert!(err.message().contains("2 of 2 feeds"), "{err}");
}

#[tokio::test(start_paused = true)]
async fn idle_feeds_go_before_a_degrade_when_that_is_enough() {
    let config = BudgetConfig {
        metadata_above: 2,
        ..roomy()
    };
    let (registry, source) = registry(config);
    let mut lease = registry.subscribe(full(pods(), "a")).await.unwrap();
    let mut stream = lease.take_feed().unwrap().into_resources().unwrap();
    let held = vec![pod("a", "x", "1"), pod("a", "y", "1")];
    source
        .feed(&full(pods(), "a"))
        .send(batch(vec![Delta::Restarted(held)]));
    next(&mut stream).await.unwrap().unwrap();
    drop(lease);

    let fresh = registry.subscribe(full(pods(), "b")).await.unwrap();
    assert!(
        !fresh.is_degraded(),
        "the idle feed made room for a full feed"
    );
    assert_eq!(registry.stats().evicted, 1);
}

#[tokio::test(start_paused = true)]
async fn a_source_error_is_returned_and_frees_the_reservation() {
    let config = BudgetConfig {
        max_feeds: 1,
        ..roomy()
    };
    let (registry, source) = registry(config);
    source.refuse(full(pods(), "a"));
    let err = registry.subscribe(full(pods(), "a")).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert_eq!(registry.stats().feeds, 0);
    registry.subscribe(full(pods(), "a")).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_abandoned_subscribe_gives_its_reservation_back() {
    let config = BudgetConfig {
        max_feeds: 1,
        ..roomy()
    };
    let (registry, source) = registry(config);
    source.hang(full(pods(), "slow"));
    let abandoned = tokio::time::timeout(
        Duration::from_secs(1),
        registry.subscribe(full(pods(), "slow")),
    );
    assert!(abandoned.await.is_err(), "the open never finished");
    registry
        .subscribe(full(pods(), "a"))
        .await
        .expect("the reservation was released");
}

#[tokio::test(start_paused = true)]
async fn a_new_config_applies_to_the_next_admission() {
    let (registry, _source) = registry(roomy());
    let _a = registry.subscribe(full(pods(), "a")).await.unwrap();
    registry.set_config(BudgetConfig {
        max_feeds: 1,
        ..roomy()
    });
    assert_eq!(registry.config().max_feeds, 1);
    let err = registry.subscribe(full(pods(), "b")).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::BudgetExceeded);
    assert_eq!(registry.stats().max_feeds, 1);
}

#[test]
fn subscribing_outside_a_runtime_is_an_internal_error() {
    let (registry, _source) = registry(roomy());
    let err = futures::executor::block_on(registry.subscribe(full(pods(), "a"))).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Internal);
}
