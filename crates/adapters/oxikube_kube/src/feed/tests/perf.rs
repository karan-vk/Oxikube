//! Throughput of the feed pipeline (watch events in, batches out) at 10k-pod scale, against
//! the scripted server on a real clock. Ignored by default; run it for the PR's perf numbers:
//!
//! ```text
//! cargo test -p oxikube_kube --release --lib feed::tests::perf -- --ignored --nocapture
//! ```

use std::time::Instant;

use oxikube_domain::session::WatchScope;
use oxikube_ports::WatchOptions;

use super::server::*;
use super::*;

const PODS_IN_LIST: usize = 10_000;
const EVENTS: usize = 50_000;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "perf measurement; run with --release --ignored --nocapture"]
async fn ten_thousand_pods_and_fifty_thousand_events() {
    let server = FeedServer::new(31);
    let items = (0..PODS_IN_LIST)
        .map(|p| pod(&format!("p{p}"), &format!("u{p}"), &(p + 1).to_string()))
        .collect();
    server.list(PODS, pod_list(items, &PODS_IN_LIST.to_string()));
    let events = (0..EVENTS)
        .map(|e| {
            let p = e % PODS_IN_LIST;
            let rv = (PODS_IN_LIST + 1 + e).to_string();
            event("MODIFIED", pod(&format!("p{p}"), &format!("u{p}"), &rv))
        })
        .collect();
    server.watch(PODS, Reply::Events(events));

    let started = Instant::now();
    let mut feed = server
        .resources(config())
        .reflector_feed(
            &pod_gvk(),
            &WatchScope::Namespaces(vec!["default".into()]),
            &WatchOptions::default(),
        )
        .await
        .unwrap();
    let mut folded = Folded::default();
    folded.apply(next_batch(&mut feed).await);
    let warm = started.elapsed();
    assert_eq!(folded.0.len(), PODS_IN_LIST);

    let streaming = Instant::now();
    let last_rv = (PODS_IN_LIST + EVENTS).to_string();
    let (mut batches, mut deltas) = (0usize, 0usize);
    while folded.0.values().all(|rv| *rv != last_rv) {
        let batch = next_batch(&mut feed).await;
        batches += 1;
        deltas += batch.len();
        folded.apply(batch);
    }
    let elapsed = streaming.elapsed();
    let per_sec = EVENTS as f64 / elapsed.as_secs_f64();
    eprintln!(
        "feed perf: initial list of {PODS_IN_LIST} pods to first batch {warm:?}; \
         {EVENTS} events in {elapsed:?} ({per_sec:.0} events/s in); \
         {batches} batches out, {deltas} deltas, mean batch {:.0} deltas",
        deltas as f64 / batches.max(1) as f64,
    );
}
