//! [`OverviewFeed`]: follows a cluster's pods, nodes and live metrics and publishes an
//! [`OverviewSnapshot`] after every batch of changes.
//!
//! The work is a future (the driver) the host runs off the UI thread and drops to stop: it owns
//! the store subscriptions and the metrics interests, so dropping it releases every feed and stops
//! the polling it asked for. The view keeps the [`OverviewFeed`] handle, which only reads.

use std::sync::Arc;

use futures::channel::mpsc;
use futures::future::BoxFuture;
use futures::stream::{BoxStream, select_all};
use futures::{FutureExt as _, StreamExt as _};
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::session::WatchScope;
use parking_lot::Mutex;

use super::distribution::{Distribution, detect_distribution};
use super::mirror::Mirror;
use super::nodes::NodeTally;
use super::pods::PodTally;
use super::snapshot::{OverviewSnapshot, UsageTotals};
use super::top::{TopConsumers, top_consumers};
use crate::metrics::{ClusterUsage, MetricsInterest, MetricsKind, MetricsService, MetricsState};
use crate::store::{FeedKind, ResourceStore, StoreDelta, StoreQuery, Subscription};

/// Rows kept in each top list.
pub const TOP_N: usize = 5;

/// What a feed follows.
#[derive(Clone)]
pub struct OverviewInput {
    /// The cluster.
    pub cluster: ClusterId,
    /// Its session's store: the Pod and Node feeds (cluster-wide).
    pub store: ResourceStore,
    /// The live metrics service, when the app has one: the feed watches node and pod metrics
    /// while it runs.
    pub metrics: Option<MetricsService>,
    /// Rows per top list.
    pub top_n: usize,
}

struct Shared {
    snapshot: Mutex<Arc<OverviewSnapshot>>,
    version: Mutex<Option<String>>,
    control: mpsc::UnboundedSender<()>,
}

/// The reading side of a cluster overview's feed: the last published snapshot.
#[derive(Clone)]
pub struct OverviewFeed {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for OverviewFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OverviewFeed").finish_non_exhaustive()
    }
}

/// A started feed: the handle, the driver to run, and a stream that yields after publishes.
pub struct StartedFeed {
    /// The handle the view reads.
    pub feed: OverviewFeed,
    /// The driver; run it off the UI thread, drop it to stop (and release the feeds).
    pub driver: BoxFuture<'static, ()>,
    /// Yields after a publish; publishes the reader has not taken yet are folded into one item.
    pub changes: BoxStream<'static, ()>,
}

impl OverviewFeed {
    /// Subscribes to `input`'s Pod and Node feeds (cluster-wide, through the store, so the watch
    /// budget decides) and registers interest in its node and pod metrics. Nothing is counted until
    /// the returned driver is polled; dropping it releases all of it.
    pub fn start(input: OverviewInput) -> StartedFeed {
        let (control, control_rx) = mpsc::unbounded();
        let (notify, changes) = mpsc::channel(0);
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Arc::new(OverviewSnapshot::default())),
            version: Mutex::new(None),
            control,
        });
        let subscribe = |kind: &str| {
            let gvk = Gvk::new("", "v1", kind);
            input
                .store
                .subscribe(StoreQuery::new(gvk, WatchScope::Cluster))
        };
        let (pods, nodes) = (subscribe("Pod"), subscribe("Node"));
        let metrics = input.metrics.as_ref().map(|service| Metrics {
            usage: service.usage(&input.cluster),
            changes: service.changes(&input.cluster),
            interests: [
                service.watch(&input.cluster, MetricsKind::Nodes),
                service.watch(&input.cluster, MetricsKind::Pods),
            ],
        });
        let driver = Driver {
            shared: shared.clone(),
            top_n: input.top_n,
            pods: Mirror::default(),
            nodes: Mirror::default(),
            pod_tally: PodTally::default(),
            node_tally: NodeTally::default(),
            usage: None,
            top: TopConsumers::default(),
            states: Default::default(),
            distribution: Distribution::Unknown,
            metrics_epoch: None,
            generation: 0,
            notify,
        };
        StartedFeed {
            feed: OverviewFeed { shared },
            driver: driver.run(pods, nodes, metrics, control_rx).boxed(),
            changes: changes.boxed(),
        }
    }

    /// The last published snapshot.
    pub fn snapshot(&self) -> Arc<OverviewSnapshot> {
        self.shared.snapshot.lock().clone()
    }

    /// Tells the feed the API server's version (`gitVersion`), one input of the distribution
    /// guess; the feed publishes again with it.
    pub fn set_server_version(&self, git_version: Option<String>) {
        *self.shared.version.lock() = git_version;
        // Fails only once the driver is gone, when nobody reads the guess any more.
        let _ = self.shared.control.unbounded_send(());
    }
}

/// The metrics side of the driver: the cluster's samples, their change stream and the interests
/// that keep them polled while the driver lives.
struct Metrics {
    usage: Arc<ClusterUsage>,
    changes: BoxStream<'static, ()>,
    interests: [MetricsInterest; 2],
}

enum Event {
    Pods(StoreDelta),
    Nodes(StoreDelta),
    Metrics,
    Version,
}

struct Driver {
    shared: Arc<Shared>,
    top_n: usize,
    pods: Mirror,
    nodes: Mirror,
    pod_tally: PodTally,
    node_tally: NodeTally,
    usage: Option<UsageTotals>,
    top: TopConsumers,
    /// Node and pod metrics states.
    states: [MetricsState; 2],
    distribution: Distribution,
    /// The usage's epoch when it was last read.
    metrics_epoch: Option<u64>,
    generation: u64,
    notify: mpsc::Sender<()>,
}

impl Driver {
    async fn run(
        mut self,
        pods: Subscription,
        nodes: Subscription,
        metrics: Option<Metrics>,
        control: mpsc::UnboundedReceiver<()>,
    ) {
        let partial = [
            pods.feed_kind() != FeedKind::Full,
            nodes.feed_kind() != FeedKind::Full,
        ];
        let mut streams: Vec<BoxStream<'static, Event>> = vec![
            pods.map(Event::Pods).boxed(),
            nodes.map(Event::Nodes).boxed(),
            control.map(|()| Event::Version).boxed(),
        ];
        let mut usage = None;
        let mut interests = None;
        if let Some(metrics) = metrics {
            streams.push(metrics.changes.map(|()| Event::Metrics).boxed());
            usage = Some(metrics.usage);
            interests = Some(metrics.interests);
        }
        let usage = usage.as_deref();
        let mut events = select_all(streams);
        // The first publish says what the metrics already hold.
        self.read_metrics(usage);
        self.publish();
        while let Some(event) = events.next().await {
            self.apply(event, partial, usage);
            // Take everything already queued, then publish once.
            while let Some(Some(event)) = events.next().now_or_never() {
                self.apply(event, partial, usage);
            }
            // Samples written without a poll's change (a reconnect clearing them) are read too.
            if usage.is_some_and(|u| Some(u.epoch()) != self.metrics_epoch) {
                self.read_metrics(usage);
            }
            self.publish();
        }
        drop(interests);
    }

    fn apply(&mut self, event: Event, partial: [bool; 2], usage: Option<&ClusterUsage>) {
        match event {
            Event::Pods(delta) => self.pods.apply(delta, partial[0], &mut self.pod_tally),
            Event::Nodes(delta) => {
                self.nodes.apply(delta, partial[1], &mut self.node_tally);
                self.redetect();
            }
            Event::Metrics => self.read_metrics(usage),
            Event::Version => self.redetect(),
        }
    }

    /// Guesses the distribution again from the server version and the nodes.
    fn redetect(&mut self) {
        let version = self.shared.version.lock().clone();
        let nodes = self.nodes.rows().iter().filter_map(|o| o.resource());
        self.distribution = detect_distribution(version.as_deref(), nodes);
    }

    /// Reads the usage totals, the top consumers and the poll states.
    fn read_metrics(&mut self, usage: Option<&ClusterUsage>) {
        let Some(usage) = usage else {
            return;
        };
        self.metrics_epoch = Some(usage.epoch());
        self.states = [
            usage.state(MetricsKind::Nodes),
            usage.state(MetricsKind::Pods),
        ];
        let mut totals = UsageTotals::default();
        usage.visit_nodes(|_, point| {
            totals.cpu_nanocores = totals
                .cpu_nanocores
                .saturating_add(point.cpu_nanocores.unwrap_or(0));
            totals.memory_bytes = totals
                .memory_bytes
                .saturating_add(point.memory_bytes.unwrap_or(0));
            totals.nodes += 1;
        });
        self.usage = (self.states[0].is_available() && totals.nodes > 0).then_some(totals);
        self.top = if self.states[1].is_available() {
            top_consumers(usage, self.top_n)
        } else {
            TopConsumers::default()
        };
    }

    /// Whether `current` already says what the tallies say now: a batch that moved nothing shown
    /// (status heartbeats, `resourceVersion` bumps) is not published. Compared field by field, so
    /// a quiet batch allocates nothing.
    fn unchanged(&self, current: &OverviewSnapshot) -> bool {
        let OverviewSnapshot {
            pods_feed,
            phases,
            demand,
            nodes_feed,
            nodes,
            capacity,
            distribution,
            node_metrics,
            pod_metrics,
            usage,
            top,
            generation: _,
        } = current;
        pods_feed == self.pods.status()
            && *phases == self.pod_tally.phases()
            && *demand == self.pod_tally.demand()
            && nodes_feed == self.nodes.status()
            && *nodes == self.node_tally.counts()
            && *capacity == self.node_tally.capacity()
            && *distribution == self.distribution
            && *node_metrics == self.states[0]
            && *pod_metrics == self.states[1]
            && *usage == self.usage
            && *top == self.top
    }

    fn publish(&mut self) {
        // The first publish always goes out: a restarted feed replaces the numbers a view kept.
        if self.generation > 0 && self.unchanged(&self.shared.snapshot.lock()) {
            return;
        }
        self.generation += 1;
        let snapshot = OverviewSnapshot {
            pods_feed: self.pods.status().clone(),
            phases: self.pod_tally.phases(),
            demand: self.pod_tally.demand(),
            nodes_feed: self.nodes.status().clone(),
            nodes: self.node_tally.counts(),
            capacity: self.node_tally.capacity(),
            distribution: self.distribution,
            node_metrics: self.states[0].clone(),
            pod_metrics: self.states[1].clone(),
            usage: self.usage,
            top: self.top.clone(),
            generation: self.generation,
        };
        *self.shared.snapshot.lock() = Arc::new(snapshot);
        // Full means a wake is already pending: the reader sees this publish then.
        let _ = self.notify.try_send(());
    }
}
