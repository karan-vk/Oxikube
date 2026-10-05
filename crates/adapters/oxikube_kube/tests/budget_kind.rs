//! Kind integration for E04-S13: the watch budget over real feeds. A namespace set opens one
//! feed per namespace and a selection change keeps, starts and (after the grace period) stops
//! them; the object budget degrades a full feed to metadata-only, then refuses; the counters
//! match what arrived. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly
//! otherwise.
//!
//! Every object is a labelled ConfigMap in the test's own `oxi-test-<rand>` namespaces, and
//! every feed selects on the label, so concurrent tests and the namespaces' own
//! `kube-root-ca.crt` never show up.
#![cfg(feature = "integration")]

mod common;

use std::time::{Duration, Instant};

use futures::StreamExt;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::api::PostParams;
use kube::{Api, Client};
use oxikube_domain::ids::{ClusterId, Gvk, Scope};
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{ErrorKind, OxiResult, Resource};
use oxikube_kube::{BudgetConfig, FeedRegistry, FeedRequest, FeedStream, ScopeChange};
use oxikube_ports::{Delta, DeltaBatch, FeedVariant, WatchFeed};
use oxikube_testkit::integration::TestNamespace;
use serde_json::json;

use common::resources::adapter;

const LABEL: &str = "oxikube.test/budget=yes";

fn config_maps() -> Gvk {
    Gvk::new("", "v1", "ConfigMap")
}

/// `count` labelled ConfigMaps `cm-<i>` in `namespace`.
async fn create_config_maps(client: &Client, namespace: &str, count: usize) {
    let api = Api::<ConfigMap>::namespaced(client.clone(), namespace);
    for i in 0..count {
        create_config_map(&api, &format!("cm-{i}")).await;
    }
}

async fn create_config_map(api: &Api<ConfigMap>, name: &str) {
    let cm: ConfigMap = serde_json::from_value(json!({
        "metadata": {"name": name, "labels": {"oxikube.test/budget": "yes"}},
        "data": {"key": "value"},
    }))
    .unwrap();
    api.create(&PostParams::default(), &cm)
        .await
        .expect("create a config map");
}

fn registry(kind: &common::Kind, client: &Client, config: BudgetConfig) -> FeedRegistry {
    let cluster = ClusterId::new("kind", &kind.context);
    FeedRegistry::for_resources(cluster, adapter(client), config)
}

fn resources(stream: FeedStream) -> WatchFeed<Resource> {
    stream.into_resources().expect("a resource feed")
}

async fn next(feed: &mut WatchFeed<Resource>) -> Option<OxiResult<DeltaBatch<Resource>>> {
    tokio::time::timeout(Duration::from_secs(60), feed.next())
        .await
        .expect("a feed item in time")
}

/// The opening list of `feed`.
async fn first_list(feed: &mut WatchFeed<Resource>) -> Vec<Resource> {
    let batch = next(feed).await.expect("open").expect("the opening batch");
    match batch.deltas.into_iter().next() {
        Some(Delta::Restarted(all)) => all,
        other => panic!("the first delta is the list, got {other:?}"),
    }
}

/// Polls `check` until it holds or 30 s pass.
async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn a_namespace_set_opens_one_feed_per_namespace_and_follows_the_selection() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let client = kind.admin_client().await;
    let a = TestNamespace::create(kind.context.as_str()).expect("namespace a");
    let b = TestNamespace::create(kind.context.as_str()).expect("namespace b");
    let c = TestNamespace::create(kind.context.as_str()).expect("namespace c");
    for ns in [&a, &b, &c] {
        create_config_maps(&client, ns.name(), 3).await;
    }
    let budget = registry(
        &kind,
        &client,
        BudgetConfig {
            idle_grace: Duration::from_secs(2),
            ..BudgetConfig::default()
        },
    );
    let template = FeedRequest::new(config_maps(), FeedVariant::Full).labels(LABEL);

    let mut selection = budget
        .subscribe_selection(
            template,
            Scope::Namespaced,
            &NamespaceSelection::from_names([a.name(), b.name()]),
        )
        .await
        .expect("subscribe {a, b}");
    let mut feeds: Vec<_> = selection
        .take_feeds()
        .into_iter()
        .map(|(namespace, stream)| (namespace, resources(stream)))
        .collect();
    assert_eq!(feeds.len(), 2, "one feed per namespace");
    for (namespace, feed) in &mut feeds {
        let listed = first_list(feed).await;
        assert_eq!(listed.len(), 3, "{namespace:?}");
        assert!(
            listed
                .iter()
                .all(|cm| cm.namespace() == namespace.as_deref())
        );
    }
    let stats = budget.stats();
    assert_eq!((stats.feeds, stats.objects, stats.restarts), (2, 6, 2));
    assert!(stats.bytes > 0, "list and watch bodies are counted");

    // {a, b} -> {b, c}: a is released, b kept, c started.
    let change = selection
        .reselect(&NamespaceSelection::from_names([b.name(), c.name()]))
        .await
        .expect("reselect {b, c}");
    let some = |ns: &TestNamespace| vec![Some(ns.name().to_owned())];
    assert_eq!(
        change,
        ScopeChange {
            start: some(&c),
            keep: some(&b),
            stop: some(&a),
        }
    );
    let (_, c_stream) = selection.take_feeds().pop().expect("c's feed");
    let mut c_feed = resources(c_stream);
    assert_eq!(first_list(&mut c_feed).await.len(), 3);

    // A change in c arrives on c's feed and is counted.
    let c_api = Api::<ConfigMap>::namespaced((*client).clone(), c.name());
    create_config_map(&c_api, "late").await;
    let batch = next(&mut c_feed).await.expect("open").expect("a batch");
    assert!(
        batch
            .deltas
            .iter()
            .any(|d| matches!(d, Delta::Applied(cm) if cm.name() == "late"))
    );
    assert!(budget.stats().events >= 1);

    // a's feed stops once its grace period is over; its consumer stream ends.
    eventually("a's feed to stop", || budget.stats().feeds == 2).await;
    let a_index = feeds
        .iter()
        .position(|(ns, _)| ns.as_deref() == Some(a.name()))
        .expect("a's stream");
    let (_, mut a_feed) = feeds.swap_remove(a_index);
    loop {
        match next(&mut a_feed).await {
            None => break,
            Some(Ok(_)) => {}
            Some(Err(err)) => panic!("unexpected error on a's feed: {err}"),
        }
    }
    let stats = budget.stats();
    assert_eq!((stats.feeds, stats.stopped, stats.objects), (2, 1, 7));
    let watched: Vec<_> = stats
        .per_feed
        .iter()
        .map(|f| f.namespace.clone().unwrap_or_default())
        .collect();
    assert!(watched.contains(&b.name().to_owned()) && watched.contains(&c.name().to_owned()));
}

#[tokio::test]
async fn the_object_budget_degrades_to_metadata_then_refuses() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    create_config_maps(&client, ns.name(), 3).await;
    let budget = registry(
        &kind,
        &client,
        BudgetConfig {
            metadata_above: 3,
            max_objects: 4,
            ..BudgetConfig::default()
        },
    );
    let all = FeedRequest::new(config_maps(), FeedVariant::Full)
        .in_namespace(Some(ns.name()))
        .labels(LABEL);

    let mut full = budget.subscribe(all.clone()).await.expect("the full feed");
    assert!(!full.is_degraded());
    let mut full_feed = resources(full.take_feed().unwrap());
    let listed = first_list(&mut full_feed).await;
    assert_eq!(listed.len(), 3);
    assert!(
        listed
            .iter()
            .all(|cm| !cm.is_partial() && cm.get("/data").is_some())
    );

    // Three objects held: the next full feed is metadata-only, with no data.
    let one = all.clone().fields("metadata.name=cm-0");
    let mut degraded = budget.subscribe(one).await.expect("a degraded feed");
    assert!(degraded.is_degraded());
    let mut degraded_feed = resources(degraded.take_feed().unwrap());
    let listed = first_list(&mut degraded_feed).await;
    assert_eq!(listed.len(), 1);
    assert!(listed[0].is_partial() && listed[0].get("/data").is_none());

    // Four held: refused, and nothing was opened for it.
    let err = budget
        .subscribe(all.fields("metadata.name=cm-1"))
        .await
        .expect_err("over the object budget");
    assert_eq!(err.kind(), ErrorKind::BudgetExceeded);
    assert!(err.message().contains("4 objects (limit 4)"), "{err}");
    let stats = budget.stats();
    assert_eq!(
        (stats.feeds, stats.objects, stats.degraded, stats.refused),
        (2, 4, 1, 1)
    );
}
