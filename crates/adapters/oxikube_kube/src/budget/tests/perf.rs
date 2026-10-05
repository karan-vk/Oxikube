//! What the budget adds on the hot path: the driver hop and the counting, at 10k-pod scale,
//! and how fast an unsubscribed feed is gone. Fake feeds on a real clock. Ignored by default;
//! run it for the PR's perf numbers:
//!
//! ```text
//! cargo test -p oxikube_kube --release --lib budget::tests::perf -- --ignored --nocapture
//! ```

use std::time::Instant;

use super::*;

const PODS: usize = 10_000;
const EVENTS: usize = 50_000;
const BATCH: usize = 1_000;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "perf measurement; run with --release --ignored --nocapture"]
async fn ten_thousand_pods_through_the_budget() {
    let config = BudgetConfig {
        max_objects: 1_000_000,
        metadata_above: 1_000_000,
        idle_grace: Duration::ZERO,
        ..roomy()
    };
    let (registry, source) = registry(config);
    let mut lease = registry.subscribe(full(pods(), "a")).await.unwrap();
    let mut stream = lease.take_feed().unwrap().into_resources().unwrap();
    let fake = source.feed(&full(pods(), "a"));
    let listed: Vec<_> = (0..PODS).map(|i| pod("a", &format!("p{i}"), "1")).collect();
    let churn: Vec<_> = (0..EVENTS / BATCH)
        .map(|b| {
            batch(
                (0..BATCH)
                    .map(|i| Delta::Applied(pod("a", &format!("p{}", (b * BATCH + i) % PODS), "2")))
                    .collect(),
            )
        })
        .collect();

    let started = Instant::now();
    fake.send(batch(vec![Delta::Restarted(listed)]));
    next(&mut stream).await.unwrap().unwrap();
    let list = started.elapsed();
    let streaming = Instant::now();
    for item in churn {
        fake.send(item);
        next(&mut stream).await.unwrap().unwrap();
    }
    let churned = streaming.elapsed();
    let stats = registry.stats();
    assert_eq!((stats.objects, stats.events), (PODS as u64, EVENTS as u64));

    let teardown = Instant::now();
    drop(lease);
    assert!(next(&mut stream).await.is_none());
    let gone = teardown.elapsed();
    assert!(!fake.is_alive());

    eprintln!(
        "budget: list of {PODS} through the driver {list:?}; {EVENTS} events in batches of \
         {BATCH}: {churned:?} ({:.0} ns/event); teardown after the last lease (zero grace): \
         {gone:?}; stats: {} feeds, {} objects",
        churned.as_nanos() as f64 / EVENTS as f64,
        stats.feeds,
        stats.objects,
    );
}
