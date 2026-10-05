//! E04-S13 observability: a feed's life under the watch budget shows up as tracing events in a
//! `feed` span (start, relist, idle, stop, degrade, refusal) and never carries object contents.
//! Runs without a cluster: the feed source is a hand-driven fake built on the public API.

mod support;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::{ErrorKind, OxiResult, Resource};
use oxikube_kube::{BudgetConfig, ByteCounter, FeedRegistry, FeedRequest, FeedSource, FeedStream};
use oxikube_ports::{Delta, DeltaBatch, FeedVariant};
use parking_lot::Mutex;
use serde_json::json;

const SECRET: &str = "s3cr3t-annotation-value";

/// Opens resource feeds whose senders the test keeps.
#[derive(Default)]
struct Source(Mutex<Vec<UnboundedSender<OxiResult<DeltaBatch<Resource>>>>>);

#[async_trait]
impl FeedSource for Source {
    async fn open(&self, _: &FeedRequest, bytes: ByteCounter) -> OxiResult<FeedStream> {
        let (tx, rx) = unbounded();
        self.0.lock().push(tx);
        bytes.add(100);
        Ok(FeedStream::Resources(Box::pin(rx)))
    }
}

fn pod(name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": "team-a", "uid": "u1", "resourceVersion": "1",
                     "annotations": {"note": SECRET}},
        "data": {"password": SECRET},
    }))
    .expect("pod")
}

fn pods(variant: FeedVariant) -> FeedRequest {
    FeedRequest::new(Gvk::new("", "v1", "Pod"), variant).in_namespace(Some("team-a"))
}

fn registry(config: BudgetConfig) -> (FeedRegistry, Arc<Source>) {
    let source = Arc::new(Source::default());
    let cluster = ClusterId::new("test", &ContextName::from("kind-trace"));
    (FeedRegistry::new(cluster, source.clone(), config), source)
}

#[tokio::test]
async fn a_feed_life_is_traced_without_object_contents() {
    support::init_tracing();
    let config = BudgetConfig {
        max_feeds: 1,
        idle_grace: Duration::ZERO,
        ..BudgetConfig::default()
    };
    let (budget, source) = registry(config);

    let mut lease = budget.subscribe(pods(FeedVariant::Full)).await.unwrap();
    let mut stream = lease.take_feed().unwrap().into_resources().unwrap();
    let tx = source.0.lock()[0].clone();
    tx.unbounded_send(Ok(DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
        pod("web-0"),
    ])])))
    .unwrap();
    tx.unbounded_send(Ok(DeltaBatch::from_deltas(vec![Delta::Applied(pod(
        "web-1",
    ))])))
    .unwrap();
    for _ in 0..2 {
        let batch = tokio::time::timeout(Duration::from_secs(10), stream.next())
            .await
            .expect("a batch in time")
            .expect("open")
            .expect("a batch");
        assert!(!batch.is_empty());
    }

    // The feed cap is 1 and the only feed is in use: refused.
    let refused = budget
        .subscribe(pods(FeedVariant::Table))
        .await
        .unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::BudgetExceeded);

    let stats = budget.stats();
    assert_eq!((stats.objects, stats.events, stats.bytes), (2, 1, 100));
    drop(lease);
    assert!(
        stream.next().await.is_none(),
        "torn down at once with no grace"
    );

    // A full request over the metadata threshold is degraded.
    let (degrading, _source) = registry(BudgetConfig {
        metadata_above: 0,
        ..BudgetConfig::default()
    });
    let lease = degrading.subscribe(pods(FeedVariant::Full)).await.unwrap();
    assert!(lease.is_degraded());

    let logs = support::captured();
    for expected in [
        "feed started",
        "feed relisted",
        "feed stopped",
        "watch budget refused a feed",
        "metadata-only instead of full objects",
        "namespace=\"team-a\"",
        "variant=\"full\"",
    ] {
        assert!(logs.contains(expected), "missing {expected:?} in:\n{logs}");
    }
    support::assert_no_secrets("budget tracing", &logs, &[SECRET, "web-0", "web-1"]);
}
