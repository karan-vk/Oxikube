//! `@metrics/pod/<ns>/<name>` over fake sessions and the live metrics ring buffers.

use oxikube_domain::ErrorKind;
use oxikube_domain::metrics::MissingReason;
use oxikube_ports::{ContextScope, MetricsOutcome};

use crate::metrics::live::MetricsKind;
use crate::tools::metrics::tests::{
    Harness, at, available, cluster, node_sample, pod_object, pod_sample,
};

/// Feeds `n` polls of `shop/web-0` into the ring buffer, 15 s apart.
fn poll(h: &Harness, n: i64) {
    let usage = h.live.usage(&cluster());
    for i in 0..n {
        let sample = pod_sample("shop", "web-0", at(15 * i), 50 + i as u64, 64 + i as u64);
        usage.apply(MetricsKind::Pods, at(15 * i), &Ok(available(vec![sample])));
    }
}

async fn resolve(
    h: &Harness,
    mention: &str,
    scope: &ContextScope,
) -> Vec<oxikube_domain::agent::ContextBlock> {
    h.contexts
        .resolve_text(mention, scope)
        .await
        .expect("resolves")
}

#[tokio::test]
async fn a_watched_pod_renders_its_last_samples_requests_and_percents() {
    let h = Harness::new();
    h.connect().await;
    h.objects().insert(pod_object("shop", "web-0"));
    poll(&h, 30);

    let blocks = resolve(&h, "@metrics/pod/shop/web-0", &ContextScope::new()).await;
    let [block] = &blocks[..] else {
        panic!("one block")
    };
    assert_eq!(block.title, "Metrics pod shop/web-0 (20 samples)");
    assert_eq!(block.mime, "text/plain");
    let body = &block.body;
    assert!(body.contains("# pod: shop/web-0"), "{body}");
    assert!(
        body.contains("# source: metrics-server, as polled"),
        "{body}"
    );
    assert!(
        body.contains("# samples: 30 from"),
        "the whole ring is counted: {body}"
    );
    assert!(
        body.contains("# requests: cpu 100m, memory 128Mi; limits: cpu 500m, memory 256Mi"),
        "{body}"
    );
    // The newest sample: 79m of a 100m request is 79%, of a 500m limit 15.8%.
    assert!(
        body.contains("# now: cpu 79m (79.0% of requests, 15.8% of limits), memory 93Mi (72.7% of requests, 36.3% of limits)"),
        "{body}"
    );
    assert!(body.contains("# note: 10 older samples left out (only the newest 20 are quoted)"));
    assert!(body.contains("\t79m\t93Mi\n"), "the newest sample: {body}");
    assert!(!body.contains("\t59m\t"), "older than the window: {body}");
    assert!(body.contains("app\t79m\t93Mi"), "the containers: {body}");
    assert!(block.truncated, "samples were left out");
}

#[tokio::test]
async fn a_cluster_nobody_watches_gets_one_reading_and_says_there_is_no_history() {
    let h = Harness::new();
    h.connect().await;
    h.objects().insert(pod_object("shop", "web-0"));
    h.metrics().script().pod_metrics.push_ok(available(vec![
        pod_sample("shop", "other", at(0), 1, 1),
        pod_sample("shop", "web-0", at(5), 120, 100),
    ]));
    let blocks = resolve(&h, "@metrics/pod/shop/web-0", &ContextScope::new()).await;
    let body = &blocks[0].body;
    assert!(body.contains("one reading now"), "{body}");
    assert!(body.contains("no history"), "{body}");
    assert!(body.contains("\t120m\t100Mi\n"), "{body}");
    assert!(!body.contains("other"), "only the mentioned pod: {body}");
    assert_eq!(
        h.live.state(&cluster(), MetricsKind::Pods),
        Default::default(),
        "the cache is untouched"
    );
}

#[tokio::test]
async fn an_unknown_pod_is_a_clear_error_block() {
    let h = Harness::new();
    h.connect().await;
    let blocks = resolve(&h, "@metrics/pod/shop/ghost", &ContextScope::new()).await;
    let [block] = &blocks[..] else {
        panic!("one block")
    };
    assert_eq!(block.title, "Metrics pod shop/ghost: unavailable");
    assert!(
        block
            .body
            .contains("# error: pod shop/ghost was not found in"),
        "{}",
        block.body
    );
    assert!(
        h.metrics().recorded_calls().is_empty(),
        "no metrics call for a pod that is not there"
    );
}

#[tokio::test]
async fn unavailable_metrics_say_so_in_the_block() {
    let h = Harness::new();
    h.connect().await;
    h.objects().insert(pod_object("shop", "web-0"));
    h.metrics()
        .script()
        .pod_metrics
        .push_ok(MetricsOutcome::Unavailable(MissingReason::NotInstalled));
    let blocks = resolve(&h, "@metrics/pod/shop/web-0", &ContextScope::new()).await;
    assert!(
        blocks[0]
            .body
            .contains("# error: metrics unavailable: metrics-server is not installed"),
        "{}",
        blocks[0].body
    );

    // A pod metrics-server has no sample for yet is named as such.
    h.metrics()
        .script()
        .pod_metrics
        .push_ok(available(vec![node_sample("n1", at(0), 1, 1)]));
    let blocks = resolve(&h, "@metrics/pod/shop/web-0", &ContextScope::new()).await;
    assert!(
        blocks[0].body.contains("has no sample for shop/web-0 yet"),
        "{}",
        blocks[0].body
    );
}

#[tokio::test]
async fn the_byte_budget_drops_the_oldest_samples_first_and_says_so() {
    let h = Harness::new();
    h.connect().await;
    h.objects().insert(pod_object("shop", "web-0"));
    poll(&h, 20);
    let tight = ContextScope::new().with_max_total_bytes(900);
    let first = resolve(&h, "@metrics/pod/shop/web-0", &tight).await;
    let second = resolve(&h, "@metrics/pod/shop/web-0", &tight).await;
    assert_eq!(first, second, "the cut is deterministic");
    let block = &first[0];
    assert!(block.body.len() <= 900, "{} bytes", block.body.len());
    assert!(block.truncated);
    assert!(
        block
            .body
            .contains("older samples left out (the size budget)"),
        "{}",
        block.body
    );
    assert!(
        block.body.contains("\t69m\t83Mi\n"),
        "the newest is kept: {}",
        block.body
    );
    assert!(
        !block.body.contains("\t50m\t"),
        "the oldest is dropped: {}",
        block.body
    );
}

#[tokio::test]
async fn a_short_mention_uses_the_threads_namespace_and_a_bad_one_is_refused() {
    let h = Harness::new();
    h.connect().await;
    h.objects().insert(pod_object("shop", "web-0"));
    poll(&h, 2);
    let scope = ContextScope::new().with_default_namespace("shop");
    let blocks = resolve(&h, "@metrics/pod/web-0", &scope).await;
    assert!(blocks[0].body.contains("# pod: shop/web-0"));

    for bad in [
        "@metrics",
        "@metrics/node/n1",
        "@metrics/pod",
        "@metrics/pod/web-0",
    ] {
        let error = h
            .contexts
            .resolve_text(bad, &ContextScope::new())
            .await
            .expect_err(bad);
        assert_eq!(error.kind(), ErrorKind::Validation, "{bad}");
        assert!(error.message().contains("@metrics"), "{}", error.message());
    }
}

#[tokio::test]
async fn a_failed_poll_keeps_the_older_samples_with_a_note() {
    let h = Harness::new();
    h.connect().await;
    h.objects().insert(pod_object("shop", "web-0"));
    poll(&h, 3);
    h.live.usage(&cluster()).apply(
        MetricsKind::Pods,
        at(100),
        &Err(oxikube_domain::OxiError::network("connection reset")),
    );
    let blocks = resolve(&h, "@metrics/pod/shop/web-0", &ContextScope::new()).await;
    assert!(
        blocks[0].body.contains(
            "# note: metrics unavailable: connection reset (the samples below are older)"
        ),
        "{}",
        blocks[0].body
    );
}
