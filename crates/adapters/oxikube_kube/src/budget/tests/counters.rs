//! The counter snapshot against what a fake feed was fed.

use oxikube_domain::ObjectMeta;
use oxikube_ports::{FeedVariant, TableRow};

use super::*;

#[tokio::test(start_paused = true)]
async fn snapshot_values_match_the_events_fed() {
    let (registry, source) = registry(roomy());
    let mut lease = registry.subscribe(full(pods(), "a")).await.unwrap();
    let mut stream = lease.take_feed().unwrap().into_resources().unwrap();
    let fake = source.feed(&full(pods(), "a"));

    let listed = vec![pod("a", "x", "1"), pod("a", "y", "1"), pod("a", "z", "1")];
    let fed = [
        batch(vec![Delta::Restarted(listed)]),
        batch(vec![
            Delta::Applied(pod("a", "w", "2")),
            Delta::Applied(pod("a", "x", "3")),
        ]),
        batch(vec![Delta::Deleted(pod("a", "y", "4"))]),
    ];
    for item in fed {
        fake.send(item);
        next(&mut stream).await.unwrap().unwrap();
    }
    fake.fail(OxiError::network("reset"));
    next(&mut stream).await.unwrap().unwrap_err();
    fake.bytes.add(1_234);

    let stats = registry.stats();
    assert_eq!(stats.feeds, 1);
    assert_eq!(stats.objects, 3, "x, z and w are held");
    assert_eq!(stats.events, 3, "two applied, one deleted");
    assert_eq!(stats.restarts, 1);
    assert_eq!(stats.errors, 1);
    assert_eq!(stats.bytes, 1_234);
    let feed = &stats.per_feed[0];
    assert_eq!(
        (feed.gvk.clone(), feed.namespace.as_deref()),
        (pods(), Some("a"))
    );
    assert_eq!(
        (feed.objects, feed.events, feed.restarts, feed.bytes),
        (3, 3, 1, 1_234)
    );
    assert_eq!((feed.variant, feed.subscribers), (FeedVariant::Full, 1));

    // A relist replaces the set; cumulative counters keep counting.
    fake.send(batch(vec![Delta::Restarted(vec![pod("a", "only", "9")])]));
    next(&mut stream).await.unwrap().unwrap();
    let stats = registry.stats();
    assert_eq!((stats.objects, stats.restarts, stats.events), (1, 2, 3));

    // Totals survive the feed: a rate across its teardown stays right.
    drop(lease);
    drop(stream);
    settle().await;
    let stats = registry.stats();
    assert_eq!((stats.feeds, stats.objects), (0, 0));
    assert_eq!((stats.events, stats.restarts, stats.bytes), (3, 2, 1_234));
    assert!(stats.per_feed.is_empty());
}

#[tokio::test(start_paused = true)]
async fn table_rows_are_counted_by_their_metadata() {
    let (registry, source) = registry(roomy());
    let request = FeedRequest::new(pods(), FeedVariant::Table).in_namespace(Some("a"));
    let mut lease = registry.subscribe(request.clone()).await.unwrap();
    let mut stream = lease.take_feed().unwrap().into_table().unwrap();
    let row = |name: &str| TableRow {
        cells: vec![json!(name)],
        meta: Some(ObjectMeta::named(name)),
        object: None,
    };
    let fake = source.feed(&request);
    fake.send_table(TableBatch {
        rows: DeltaBatch::from_deltas(vec![Delta::Restarted(vec![row("a"), row("b")])]),
        ..TableBatch::default()
    });
    fake.send_table(TableBatch {
        rows: DeltaBatch::from_deltas(vec![Delta::Applied(row("c")), Delta::Deleted(row("a"))]),
        ..TableBatch::default()
    });
    for _ in 0..2 {
        tokio::time::timeout(Duration::from_secs(60), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
    let stats = registry.stats();
    assert_eq!((stats.objects, stats.events, stats.restarts), (2, 2, 1));
    assert_eq!(stats.per_feed[0].variant, FeedVariant::Table);
}

#[tokio::test(start_paused = true)]
async fn feeds_are_listed_in_a_stable_order() {
    let (registry, _source) = registry(roomy());
    let _b = registry.subscribe(full(pods(), "b")).await.unwrap();
    let _a = registry.subscribe(full(pods(), "a")).await.unwrap();
    let _d = registry.subscribe(full(deployments(), "a")).await.unwrap();
    let order: Vec<_> = registry
        .stats()
        .per_feed
        .iter()
        .map(|f| {
            (
                f.gvk.kind.to_string(),
                f.namespace.clone().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(order.len(), 3);
    assert_eq!(order.iter().filter(|(k, _)| k == "Pod").count(), 2);
    let pods: Vec<_> = order
        .iter()
        .filter(|(k, _)| k == "Pod")
        .map(|(_, n)| n.as_str())
        .collect();
    assert_eq!(pods, ["a", "b"]);
}
