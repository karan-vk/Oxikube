//! Kind integration for E04-S02: a reflector feed under pod churn. A consumer folds the
//! delta batches while pods are created, relabelled and deleted; once the churn stops, its
//! state must equal a fresh list. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`;
//! skips cleanly otherwise.
//!
//! Pods are never scheduled (`pending_pod`) so the churn loads the API server, neither the
//! shared node nor its scheduler, and live in the test's own `oxi-test-<rand>` namespace.
#![cfg(feature = "integration")]

mod common;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use futures::{FutureExt, StreamExt, stream};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::{Api, Client};
use oxikube_domain::Resource;
use oxikube_domain::ids::Gvk;
use oxikube_domain::session::WatchScope;
use oxikube_kube::{FeedConfig, FeedState, ReflectorFeed, StreamingLists};
use oxikube_ports::{Delta, DeltaBatch, WatchOptions};
use oxikube_testkit::integration::TestNamespace;
use serde_json::json;

use common::resources::{adapter, create_pods, pending_pod};

const INITIAL_PODS: usize = 30;
const CHURN_ROUNDS: usize = 6;

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// What a consumer folding the deltas holds: `namespace/name` to resource version.
#[derive(Default)]
struct Folded {
    objects: BTreeMap<String, String>,
    batches: usize,
    deltas: usize,
    restarts: usize,
}

impl Folded {
    fn apply(&mut self, batch: DeltaBatch<Resource>) {
        self.batches += 1;
        self.deltas += batch.len();
        for delta in batch {
            match delta {
                Delta::Restarted(all) => {
                    self.restarts += 1;
                    self.objects = all.iter().map(|r| (key(r), rv(r))).collect();
                }
                Delta::Applied(r) => {
                    self.objects.insert(key(&r), rv(&r));
                }
                Delta::Deleted(r) => {
                    self.objects.remove(&key(&r));
                }
            }
        }
    }
}

fn key(r: &Resource) -> String {
    format!("{}/{}", r.namespace().unwrap_or_default(), r.name())
}

fn rv(r: &Resource) -> String {
    r.meta
        .resource_version
        .as_deref()
        .unwrap_or_default()
        .to_owned()
}

/// A fresh `namespace/name -> resourceVersion` list of the pods matching `params`.
async fn fresh_list(api: &Api<Pod>, params: &ListParams) -> BTreeMap<String, String> {
    api.list(params)
        .await
        .expect("fresh list")
        .items
        .into_iter()
        .map(|p| {
            let m = p.metadata;
            (
                format!(
                    "{}/{}",
                    m.namespace.unwrap_or_default(),
                    m.name.unwrap_or_default()
                ),
                m.resource_version.unwrap_or_default(),
            )
        })
        .collect()
}

/// Folds every batch that arrives within `quiet` of the previous one.
async fn drain(feed: &mut ReflectorFeed, folded: &mut Folded, quiet: Duration) {
    while let Ok(Some(item)) = tokio::time::timeout(quiet, feed.next()).await {
        folded.apply(item.expect("no feed error under churn"));
    }
}

/// Folds until the consumer's state equals a fresh list, or fails after `deadline`.
/// Kubernetes keeps touching pods (scheduler conditions), so the two are compared repeatedly.
async fn converge(
    feed: &mut ReflectorFeed,
    folded: &mut Folded,
    api: &Api<Pod>,
    params: &ListParams,
) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        drain(feed, folded, Duration::from_millis(500)).await;
        let fresh = fresh_list(api, params).await;
        if fresh == folded.objects {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "folded state never matched a fresh list: {} folded vs {} listed",
            folded.objects.len(),
            fresh.len()
        );
    }
}

/// `rounds` rounds of churn on the pods named `<prefix>-<i>`: each deletes three pods,
/// creates three, and relabels five, all concurrently.
async fn churn(client: &Client, namespace: &str, prefix: &str, labels: &[(&str, &str)]) {
    let api = Api::<Pod>::namespaced(client.clone(), namespace);
    let mut next = INITIAL_PODS;
    let mut alive: Vec<usize> = (0..INITIAL_PODS).collect();
    for round in 0..CHURN_ROUNDS {
        let doomed: Vec<usize> = alive.drain(..3).collect();
        let fresh: Vec<usize> = (next..next + 3).collect();
        next += 3;
        let touched: Vec<usize> = alive.iter().take(5).copied().collect();
        alive.extend(&fresh);
        let deletes = doomed.into_iter().map(|i| {
            let api = api.clone();
            let name = format!("{prefix}-{i}");
            async move {
                api.delete(&name, &DeleteParams::default().grace_period(0))
                    .await
                    .map(|_| ())
            }
            .boxed()
        });
        let creates = fresh.into_iter().map(|i| {
            let api = api.clone();
            let pod = pending_pod(&format!("{prefix}-{i}"), labels);
            async move { api.create(&PostParams::default(), &pod).await.map(|_| ()) }.boxed()
        });
        let patches = touched.into_iter().map(|i| {
            let api = api.clone();
            let name = format!("{prefix}-{i}");
            let patch = json!({"metadata": {"labels": {"round": round.to_string()}}});
            async move {
                api.patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
                    .await
                    .map(|_| ())
            }
            .boxed()
        });
        let results: Vec<_> = stream::iter(deletes.chain(creates).chain(patches))
            .buffer_unordered(16)
            .collect()
            .await;
        for result in results {
            result.expect("churn request");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Opens `scope` on the adapter with `config`, folds its opening batch and returns both.
async fn open(
    client: &Client,
    config: FeedConfig,
    scope: &WatchScope,
    options: &WatchOptions,
) -> (ReflectorFeed, Folded) {
    let resources = adapter(client).with_feed_config(config);
    let mut feed = resources
        .reflector_feed(&pod_gvk(), scope, options)
        .await
        .expect("open the feed");
    let mut folded = Folded::default();
    let first = tokio::time::timeout(Duration::from_secs(30), feed.next())
        .await
        .expect("opening batch in time")
        .expect("feed open")
        .expect("opening batch");
    assert!(
        matches!(first.deltas.first(), Some(Delta::Restarted(_))),
        "the first batch opens with Restarted"
    );
    folded.apply(first);
    (feed, folded)
}

/// Churn over one namespace with the default settings (on kind >= 1.32: a streaming list),
/// folded deltas equal a fresh list.
#[tokio::test]
async fn a_namespace_feed_under_churn_folds_to_a_fresh_list() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let labels = [("app", "feed-churn")];
    let pods = (0..INITIAL_PODS)
        .map(|i| pending_pod(&format!("churn-{i}"), &labels))
        .collect();
    create_pods(&client, ns.name(), pods).await;

    let scope = WatchScope::Namespaces(vec![ns.name().to_owned()]);
    let (mut feed, mut folded) = open(
        &client,
        FeedConfig::default(),
        &scope,
        &WatchOptions::default(),
    )
    .await;
    assert_eq!(
        folded.objects.len(),
        INITIAL_PODS,
        "the opening list holds every pod"
    );
    let state = feed.state();

    let started = Instant::now();
    let consumer = async {
        drain(&mut feed, &mut folded, Duration::from_secs(3)).await;
    };
    tokio::join!(churn(&client, ns.name(), "churn", &labels), consumer);
    let churned = started.elapsed();

    let api = Api::<Pod>::namespaced((*client).clone(), ns.name());
    converge(&mut feed, &mut folded, &api, &ListParams::default()).await;
    assert_eq!(folded.restarts, 1, "no relist under churn");
    assert_eq!(*state.borrow(), FeedState::Live);
    eprintln!(
        "feed churn: {} batches, {} deltas (mean {:.1}) in {churned:?}",
        folded.batches,
        folded.deltas,
        folded.deltas as f64 / folded.batches as f64
    );
}

/// A cluster-wide feed with a label selector and paged lists sees only the selected pods
/// and also folds to a fresh list.
#[tokio::test]
async fn a_cluster_feed_with_a_selector_and_paged_lists_folds_to_a_fresh_list() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    // Unique per run: other tests churn pods cluster-wide at the same time.
    let run = ns.name().to_owned();
    let labels = [("feed-run", run.as_str())];
    let pods = (0..INITIAL_PODS)
        .map(|i| pending_pod(&format!("sel-{i}"), &labels))
        .collect();
    create_pods(&client, ns.name(), pods).await;
    create_pods(&client, ns.name(), vec![pending_pod("unselected", &[])]).await;

    let config = FeedConfig {
        streaming_lists: StreamingLists::Never,
        page_size: 7,
        ..FeedConfig::default()
    };
    let selector = format!("feed-run={run}");
    let options = WatchOptions::default().labels(selector.clone());
    let (mut feed, mut folded) = open(&client, config, &WatchScope::Cluster, &options).await;
    assert_eq!(
        folded.objects.len(),
        INITIAL_PODS,
        "paged list, selected pods only"
    );

    let consumer = async {
        drain(&mut feed, &mut folded, Duration::from_secs(3)).await;
    };
    tokio::join!(churn(&client, ns.name(), "sel", &labels), consumer);

    let api = Api::<Pod>::all((*client).clone());
    converge(
        &mut feed,
        &mut folded,
        &api,
        &ListParams::default().labels(&selector),
    )
    .await;
    assert!(!folded.objects.keys().any(|k| k.ends_with("/unselected")));
}
