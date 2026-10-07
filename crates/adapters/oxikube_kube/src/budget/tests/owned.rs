//! Owned feeds (E04-F543): one per port call, never shared or degraded, gone with their stream,
//! and released ahead of the drop so a consumer that evicts its own idle feed is admitted.

use oxikube_domain::ErrorKind;
use oxikube_ports::FeedVariant;

use super::*;

/// Opens an owned feed of `request` on the fake source.
async fn owned(
    registry: &FeedRegistry,
    source: &Arc<FakeSource>,
    request: FeedRequest,
) -> OxiResult<WatchFeed<Resource>> {
    let opening = request.clone();
    let stream = registry
        .open_owned(request, |bytes| async move {
            source.open(&opening, bytes).await
        })
        .await?;
    Ok(stream.into_resources().expect("a resource feed"))
}

fn config_with(max_feeds: usize) -> BudgetConfig {
    BudgetConfig {
        max_feeds,
        ..roomy()
    }
}

#[tokio::test(start_paused = true)]
async fn equal_owned_feeds_are_separate_feeds_each_with_its_stream() {
    let (registry, source) = registry(roomy());
    let mut first = owned(&registry, &source, full(pods(), "a")).await.unwrap();
    let mut second = owned(&registry, &source, full(pods(), "a")).await.unwrap();
    assert_eq!(source.opened().len(), 2, "never joined");
    let feeds = source.feeds.lock().clone();
    feeds[0].send(batch(vec![Delta::Restarted(vec![pod("a", "x", "1")])]));
    feeds[1].send(batch(vec![Delta::Restarted(vec![])]));
    assert_eq!(next(&mut first).await.unwrap().unwrap().len(), 1);
    assert_eq!(next(&mut second).await.unwrap().unwrap().len(), 1);
    settle().await;
    let stats = registry.stats();
    assert_eq!((stats.feeds, stats.subscribers, stats.objects), (2, 2, 1));
}

#[tokio::test(start_paused = true)]
async fn dropping_the_stream_tears_the_feed_down_at_once() {
    let (registry, source) = registry(roomy());
    let feed = owned(&registry, &source, full(pods(), "a")).await.unwrap();
    let fake = source.feed(&full(pods(), "a"));
    drop(feed);
    let stats = registry.stats();
    assert_eq!(
        (stats.feeds, stats.stopped),
        (0, 1),
        "gone before any task ran: no grace period for an owned feed"
    );
    settle().await;
    assert!(!fake.is_alive(), "its driver was aborted");
}

#[tokio::test(start_paused = true)]
async fn owned_feeds_are_admitted_against_the_limits() {
    let (registry, source) = registry(config_with(1));
    let _held = owned(&registry, &source, full(pods(), "a")).await.unwrap();
    let err = owned(&registry, &source, full(config_maps(), "a"))
        .await
        .err()
        .expect("refused");
    assert_eq!(err.kind(), ErrorKind::BudgetExceeded);
    assert!(err.message().contains("1 of 1 feeds"), "{}", err.message());
    assert_eq!(registry.stats().refused, 1);
    assert_eq!(source.opened().len(), 1, "a refused feed is never opened");
}

#[tokio::test(start_paused = true)]
async fn an_idle_shared_feed_makes_room_for_an_owned_one() {
    let (registry, source) = registry(config_with(1));
    let lease = registry.subscribe(full(deployments(), "a")).await.unwrap();
    drop(lease);
    assert_eq!(registry.stats().idle_feeds, 1);
    let _feed = owned(&registry, &source, full(pods(), "a")).await.unwrap();
    let stats = registry.stats();
    assert_eq!((stats.feeds, stats.evicted), (1, 1));
}

#[tokio::test(start_paused = true)]
async fn an_owned_full_feed_is_never_degraded() {
    let config = BudgetConfig {
        metadata_above: 0,
        ..roomy()
    };
    let (registry, source) = registry(config);
    assert_eq!(
        registry.check(&full(pods(), "a")).unwrap(),
        FeedVariant::Metadata,
        "the check tells a consumer it may degrade"
    );
    let _feed = owned(&registry, &source, full(pods(), "a")).await.unwrap();
    assert_eq!(source.opened(), vec![full(pods(), "a")]);
    assert_eq!(registry.stats().degraded, 0);
}

#[tokio::test(start_paused = true)]
async fn a_released_feed_stops_counting_before_its_stream_drops() {
    let (registry, source) = registry(config_with(1));
    let first = owned(&registry, &source, full(pods(), "a")).await.unwrap();
    let next_one = full(config_maps(), "a");
    assert_eq!(
        registry.check(&next_one).unwrap_err().kind(),
        ErrorKind::BudgetExceeded
    );
    assert!(registry.release_owned(&full(pods(), "a")));
    assert!(!registry.release_owned(&full(pods(), "a")), "released once");
    assert_eq!(registry.check(&next_one).unwrap(), FeedVariant::Full);
    let _second = owned(&registry, &source, next_one).await.unwrap();
    assert_eq!(registry.stats().feeds, 2, "the released one still runs");
    drop(first);
    assert_eq!(registry.stats().feeds, 1);
    assert_eq!(
        registry
            .check(&full(deployments(), "a"))
            .unwrap_err()
            .kind(),
        ErrorKind::BudgetExceeded,
        "the feed still open counts"
    );
}

#[tokio::test(start_paused = true)]
async fn a_release_marks_the_equal_feed_that_actually_goes() {
    let (registry, source) = registry(config_with(2));
    let _kept = owned(&registry, &source, full(pods(), "a")).await.unwrap();
    let going = owned(&registry, &source, full(pods(), "a")).await.unwrap();
    // The release marks the oldest equal feed, but it is the other one whose stream drops.
    assert!(registry.release_owned(&full(pods(), "a")));
    drop(going);
    let _other = owned(&registry, &source, full(config_maps(), "a"))
        .await
        .unwrap();
    assert_eq!(
        registry
            .check(&full(deployments(), "a"))
            .unwrap_err()
            .kind(),
        ErrorKind::BudgetExceeded,
        "the kept feed counts again once the released one is gone"
    );
}

#[tokio::test(start_paused = true)]
async fn a_reserved_slot_counts_before_its_open_and_the_open_takes_it() {
    let (registry, source) = registry(config_with(2));
    // Two decisions in a burst, before either port call: the second sees the first's slot.
    assert_eq!(
        registry.reserve_owned(&full(pods(), "a")).unwrap(),
        FeedVariant::Full
    );
    assert_eq!(
        registry.reserve_owned(&full(pods(), "b")).unwrap(),
        FeedVariant::Full
    );
    let err = registry
        .reserve_owned(&full(config_maps(), "a"))
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::BudgetExceeded);
    assert!(err.message().contains("2 of 2 feeds"), "{}", err.message());
    // The opens take their slots: neither is refused nor counted twice.
    let _a = owned(&registry, &source, full(pods(), "a")).await.unwrap();
    let _b = owned(&registry, &source, full(pods(), "b")).await.unwrap();
    let stats = registry.stats();
    assert_eq!((stats.feeds, stats.refused), (2, 0));
    assert_eq!(
        registry
            .check(&full(config_maps(), "a"))
            .unwrap_err()
            .kind(),
        ErrorKind::BudgetExceeded
    );
}

#[tokio::test(start_paused = true)]
async fn a_slot_given_up_before_its_open_is_released() {
    let (registry, source) = registry(config_with(1));
    registry.reserve_owned(&full(pods(), "a")).unwrap();
    assert!(registry.reserve_owned(&full(pods(), "b")).is_err());
    assert!(registry.release_owned(&full(pods(), "a")), "the slot goes");
    assert!(!registry.release_owned(&full(pods(), "a")), "released once");
    registry.reserve_owned(&full(pods(), "b")).unwrap();
    // An open with no slot of its own is admitted against the held one.
    let err = owned(&registry, &source, full(pods(), "a"))
        .await
        .err()
        .expect("refused");
    assert_eq!(err.kind(), ErrorKind::BudgetExceeded);
}

#[tokio::test(start_paused = true)]
async fn a_reservation_tears_down_the_idle_shared_feed_it_needs() {
    let (registry, source) = registry(config_with(1));
    let lease = registry.subscribe(full(deployments(), "a")).await.unwrap();
    drop(lease);
    registry.reserve_owned(&full(pods(), "a")).unwrap();
    let stats = registry.stats();
    assert_eq!(
        (stats.feeds, stats.evicted),
        (0, 1),
        "evicted at the reservation"
    );
    let _feed = owned(&registry, &source, full(pods(), "a")).await.unwrap();
    assert_eq!(registry.stats().feeds, 1);
}

#[tokio::test(start_paused = true)]
async fn a_degraded_reservation_is_held_for_the_metadata_feed() {
    let config = BudgetConfig {
        max_feeds: 1,
        metadata_above: 0,
        ..roomy()
    };
    let (registry, source) = registry(config);
    assert_eq!(
        registry.reserve_owned(&full(pods(), "a")).unwrap(),
        FeedVariant::Metadata
    );
    let metadata = full(pods(), "a").with_variant(FeedVariant::Metadata);
    let _feed = owned(&registry, &source, metadata).await.unwrap();
    assert_eq!(registry.stats().feeds, 1, "the open took the slot");
}
