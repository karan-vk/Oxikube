//! Watch-budget tests. Most run against [`FakeSource`], whose feeds the test drives by hand,
//! on a paused tokio clock (grace periods pass instantly and exactly); `kube` runs the real
//! source over the in-process fake API server.

mod counters;
mod kube;
mod perf;
mod policy;
mod registry;
mod selection;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::{Stream, StreamExt};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::{Delta, DeltaBatch, FeedVariant, TableBatch, WatchFeed};
use parking_lot::Mutex;
use serde_json::json;

use super::{BudgetConfig, ByteCounter, FeedRegistry, FeedRequest, FeedSource, FeedStream};

fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::from("kind-test"))
}

fn pods() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

fn deployments() -> Gvk {
    Gvk::new("apps", "v1", "Deployment")
}

fn config_maps() -> Gvk {
    Gvk::new("", "v1", "ConfigMap")
}

/// A full feed of `gvk` in `namespace`.
fn full(gvk: Gvk, namespace: &str) -> FeedRequest {
    FeedRequest::new(gvk, FeedVariant::Full).in_namespace(Some(namespace))
}

/// A pod `name` in `namespace` at resource version `rv`.
fn pod(namespace: &str, name: &str, rv: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": namespace, "uid": format!("u-{name}"), "resourceVersion": rv},
    }))
    .expect("pod")
}

/// A batch of `deltas`.
fn batch(deltas: Vec<Delta<Resource>>) -> DeltaBatch<Resource> {
    DeltaBatch::from_deltas(deltas)
}

/// Settings with a 30 s grace period and limits far away.
fn roomy() -> BudgetConfig {
    BudgetConfig {
        max_feeds: 100,
        max_objects: 1_000,
        metadata_above: 1_000,
        idle_grace: Duration::from_secs(30),
    }
}

/// A registry over a fresh fake source.
fn registry(config: BudgetConfig) -> (FeedRegistry, Arc<FakeSource>) {
    let source = Arc::new(FakeSource::default());
    let registry = FeedRegistry::new(cluster(), source.clone(), config);
    (registry, source)
}

/// The next item of a resource feed; fails if none comes within ten (virtual) minutes.
async fn next(feed: &mut WatchFeed<Resource>) -> Option<OxiResult<DeltaBatch<Resource>>> {
    tokio::time::timeout(Duration::from_secs(600), feed.next())
        .await
        .expect("a feed item or the end in time")
}

/// Lets spawned tasks (drivers, timers) run without moving the clock.
async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

/// What the test sends on a fake feed.
enum Sender {
    Resources(UnboundedSender<OxiResult<DeltaBatch<Resource>>>),
    Table(UnboundedSender<OxiResult<TableBatch>>),
}

/// The test's end of one feed the fake source opened.
#[derive(Clone)]
struct FakeFeed {
    request: FeedRequest,
    sender: Arc<Sender>,
    bytes: ByteCounter,
    alive: Arc<AtomicBool>,
}

impl FakeFeed {
    /// Sends a resource batch.
    fn send(&self, batch: DeltaBatch<Resource>) {
        match &*self.sender {
            Sender::Resources(tx) => tx.unbounded_send(Ok(batch)).expect("feed open"),
            Sender::Table(_) => panic!("not a resource feed"),
        }
    }

    /// Sends a table batch.
    fn send_table(&self, batch: TableBatch) {
        match &*self.sender {
            Sender::Table(tx) => tx.unbounded_send(Ok(batch)).expect("feed open"),
            Sender::Resources(_) => panic!("not a table feed"),
        }
    }

    /// Sends an error item.
    fn fail(&self, err: OxiError) {
        match &*self.sender {
            Sender::Resources(tx) => tx.unbounded_send(Err(err)).expect("feed open"),
            Sender::Table(tx) => tx.unbounded_send(Err(err)).expect("feed open"),
        }
    }

    /// Ends the feed, as a source does after its final error.
    fn end(&self) {
        match &*self.sender {
            Sender::Resources(tx) => tx.close_channel(),
            Sender::Table(tx) => tx.close_channel(),
        }
    }

    /// Whether the registry still holds the feed's stream (not dropped).
    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

/// A source of hand-driven feeds that records what it opened.
#[derive(Default)]
struct FakeSource {
    feeds: Mutex<Vec<FakeFeed>>,
    /// Requests whose next open fails with `Unsupported`.
    refuse: Mutex<Vec<FeedRequest>>,
    /// Requests whose opens never finish.
    hang: Mutex<Vec<FeedRequest>>,
}

impl FakeSource {
    /// Every request opened so far, oldest first.
    fn opened(&self) -> Vec<FeedRequest> {
        self.feeds
            .lock()
            .iter()
            .map(|f| f.request.clone())
            .collect()
    }

    /// The most recently opened feed for `request`.
    fn feed(&self, request: &FeedRequest) -> FakeFeed {
        self.feeds
            .lock()
            .iter()
            .rev()
            .find(|f| &f.request == request)
            .cloned()
            .unwrap_or_else(|| panic!("no feed opened for {request:?}"))
    }

    /// Makes the next open of `request` fail.
    fn refuse(&self, request: FeedRequest) {
        self.refuse.lock().push(request);
    }

    /// Makes every open of `request` hang.
    fn hang(&self, request: FeedRequest) {
        self.hang.lock().push(request);
    }
}

#[async_trait]
impl FeedSource for FakeSource {
    async fn open(&self, request: &FeedRequest, bytes: ByteCounter) -> OxiResult<FeedStream> {
        let refused = {
            let mut refuse = self.refuse.lock();
            let at = refuse.iter().position(|r| r == request);
            at.map(|at| refuse.remove(at)).is_some()
        };
        if refused {
            return Err(OxiError::unsupported("the cluster does not serve it"));
        }
        let hangs = self.hang.lock().contains(request);
        if hangs {
            std::future::pending::<()>().await;
        }
        let alive = Arc::new(AtomicBool::new(true));
        let (sender, stream) = if request.variant == FeedVariant::Table {
            let (tx, rx) = unbounded();
            let stream = Guarded(rx, alive.clone());
            (Sender::Table(tx), FeedStream::Table(Box::pin(stream)))
        } else {
            let (tx, rx) = unbounded();
            let stream = Guarded(rx, alive.clone());
            (
                Sender::Resources(tx),
                FeedStream::Resources(Box::pin(stream)),
            )
        };
        self.feeds.lock().push(FakeFeed {
            request: request.clone(),
            sender: Arc::new(sender),
            bytes,
            alive,
        });
        Ok(stream)
    }
}

/// A fake feed's stream; clears its `alive` flag when dropped (the feed is torn down).
struct Guarded<T>(UnboundedReceiver<T>, Arc<AtomicBool>);

impl<T> Stream for Guarded<T> {
    type Item = T;

    fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        self.0.poll_next_unpin(cx)
    }
}

impl<T> Drop for Guarded<T> {
    fn drop(&mut self) {
        self.1.store(false, Ordering::SeqCst);
    }
}
