//! The feed over a scripted store: the first list, churn applied incrementally (and agreeing
//! with a recompute over the store's objects), a forbidden feed, the distribution guess, and
//! dropping the driver releasing both feeds.

use futures::StreamExt as _;
use oxikube_domain::{OxiError, Resource};
use oxikube_ports::Delta;
use oxikube_testkit::{node, pod};
use serde_json::json;

use super::{Harness, batch, cluster, timeline};
use crate::cluster_overview::{
    Distribution, OverviewFeed, OverviewInput, OverviewSnapshot, PodTally, StartedFeed, TOP_N,
};
use crate::store::FeedState;

fn start(h: &Harness) -> StartedFeed {
    OverviewFeed::start(OverviewInput {
        cluster: cluster(),
        store: h.store.clone(),
        metrics: None,
        top_n: TOP_N,
    })
}

fn running(name: &str, node: &str) -> Resource {
    pod()
        .namespace("shop")
        .name(name)
        .running()
        .node(node)
        .requests("500m", "256Mi")
        .build()
}

#[test]
fn the_first_list_and_churn_reach_the_snapshot_incrementally() {
    let mut h = Harness::new();
    let initial = vec![
        running("web-1", "n1"),
        running("web-2", "n1"),
        pod()
            .namespace("shop")
            .name("job-1")
            .succeeded()
            .node("n1")
            .build(),
    ];
    let pending = pod().namespace("shop").name("web-3").pending().build();
    let scheduled = running("web-3", "n2");
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(initial.clone())]),
        batch(vec![Delta::Applied(pending)]),
        batch(vec![
            Delta::Applied(scheduled.clone()),
            Delta::Deleted(initial[0].clone()),
        ]),
    ]));
    h.resources
        .insert(node().name("n1").allocatable("4", "16Gi", "110").build());
    h.resources.insert(
        node()
            .name("n2")
            .not_ready()
            .allocatable("2", "8Gi", "110")
            .build(),
    );
    let started = start(&h);
    let feed = started.feed.clone();
    h.spawn(started.driver);
    h.settle();

    let s = feed.snapshot();
    assert!(s.pods_feed.is_listed() && s.nodes_feed.is_listed());
    assert_eq!(s.phases.running, 2);
    assert_eq!(s.phases.succeeded, 1);
    assert_eq!(s.demand.pods, 2);
    assert_eq!(s.demand.cpu_requests, 1000);
    assert_eq!(s.nodes.ready, 1);
    assert_eq!(s.nodes.not_ready, 1);
    assert_eq!(s.capacity.cpu_millis, 6000);
    assert_eq!(s.usage, None, "no metrics service: no usage, never a zero");

    h.advance(1);
    assert_eq!(feed.snapshot().phases.pending, 1);
    h.advance(1);
    let s = feed.snapshot();
    assert_eq!(s.phases.pending, 0);
    assert_eq!(s.phases.running, 2, "web-3 runs, web-1 is gone");
    let expected = PodTally::of([&initial[1], &initial[2], &scheduled]);
    assert_eq!(s.phases, expected.phases());
    assert_eq!(s.demand, expected.demand());
    assert!(s.generation > 1);
}

#[test]
fn a_forbidden_pod_feed_says_so_and_counts_nothing() {
    let mut h = Harness::new();
    h.resources.script().watch.push_err(OxiError::forbidden(
        "pods is forbidden: User cannot list pods",
    ));
    let started = start(&h);
    let feed = started.feed.clone();
    h.spawn(started.driver);
    h.settle();
    let s = feed.snapshot();
    assert!(matches!(s.pods_feed.state, FeedState::Forbidden { .. }));
    assert!(
        s.pods_feed
            .problem()
            .is_some_and(|p| p.starts_with("no access"))
    );
    assert_eq!(s.phases.total(), 0);
}

#[test]
fn the_distribution_follows_the_nodes_and_the_version() {
    let mut h = Harness::new();
    let mut kind_node = node().name("kind-control-plane").json();
    kind_node["spec"]["providerID"] = json!("kind://docker/kind/kind-control-plane");
    h.resources
        .insert(Resource::from_json(kind_node).expect("a node"));
    let started = start(&h);
    let feed = started.feed.clone();
    h.spawn(started.driver);
    h.settle();
    assert_eq!(feed.snapshot().distribution, Distribution::Kind);

    let mut h = Harness::new();
    h.resources.insert(node().name("worker").build());
    let started = start(&h);
    let feed = started.feed.clone();
    h.spawn(started.driver);
    h.settle();
    assert_eq!(feed.snapshot().distribution, Distribution::Unknown);
    feed.set_server_version(Some("v1.31.1+k3s1".to_owned()));
    h.settle();
    assert_eq!(feed.snapshot().distribution, Distribution::K3s);
}

#[test]
fn a_publish_wakes_the_reader_and_dropping_the_driver_releases_the_feeds() {
    let mut h = Harness::new();
    h.resources.insert(running("web-1", "n1"));
    let mut started = start(&h);
    let (abort, registration) = futures::future::AbortHandle::new_pair();
    h.spawn(Box::pin(async move {
        let _ = futures::future::Abortable::new(started.driver, registration).await;
    }));
    h.settle();
    assert_eq!(h.resources.live_watches(), 2, "one Pod feed, one Node feed");
    assert!(
        futures::FutureExt::now_or_never(started.changes.next()).is_some(),
        "a publish wakes the reader"
    );
    abort.abort();
    h.settle();
    h.advance(1);
    assert_eq!(
        h.resources.live_watches(),
        0,
        "the feeds stop with the driver"
    );
}

#[test]
fn a_batch_that_changes_no_number_is_not_published() {
    let mut h = Harness::new();
    let web = running("web-1", "n1");
    let relabelled = pod()
        .namespace("shop")
        .name("web-1")
        .running()
        .node("n1")
        .requests("500m", "256Mi")
        .label("heartbeat", "2")
        .resource_version("2")
        .build();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(vec![web])]),
        batch(vec![Delta::Applied(relabelled)]),
        batch(vec![Delta::Applied(
            pod().namespace("shop").name("web-2").pending().build(),
        )]),
    ]));
    h.resources.insert(node().name("n1").build());
    let mut started = start(&h);
    let feed = started.feed.clone();
    h.spawn(started.driver);
    h.settle();
    while futures::FutureExt::now_or_never(started.changes.next()).is_some() {}
    let before = feed.snapshot();
    assert_eq!(before.phases.running, 1);

    h.advance(1);
    let after = feed.snapshot();
    assert!(
        std::sync::Arc::ptr_eq(&before, &after),
        "a relabelled pod moves no number: no new snapshot"
    );
    assert!(
        futures::FutureExt::now_or_never(started.changes.next()).is_none(),
        "and no wake for the reader"
    );

    h.advance(1);
    let moved = feed.snapshot();
    assert_eq!(moved.phases.pending, 1);
    assert_eq!(moved.generation, before.generation + 1);
    assert!(futures::FutureExt::now_or_never(started.changes.next()).is_some());
}

#[test]
fn same_numbers_ignores_the_generation_only() {
    let a = OverviewSnapshot::default();
    let b = OverviewSnapshot {
        generation: 7,
        ..OverviewSnapshot::default()
    };
    assert!(a.same_numbers(&b));
    assert_ne!(a, b);
    let mut c = b;
    c.phases.running = 1;
    assert!(!a.same_numbers(&c));
}
