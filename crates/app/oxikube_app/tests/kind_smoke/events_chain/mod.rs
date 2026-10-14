//! The events chain on kind (E14-S10), below the UI: a pod whose image cannot be pulled makes
//! the kubelet write `Warning`s, the real events adapter feeds the app's `EventService`, and the
//! `NotificationService` turns them into notifications. The app-level half (the events view, the
//! table badge, the drawer's Events tab, the toast) is `bins/oxikube/tests/kind_events.rs`.
//!
//! | Scenario | Test |
//! |---|---|
//! | a Warning reaches the `EventService` within 1 s of a `kubectl` watch seeing it, on `events.k8s.io/v1` | `fallback::without_events_k8s_io_the_service_reads_core_v1_and_normalises_the_same_events` |
//! | the same with `events.k8s.io` unavailable: the feed reads `core/v1` and the normalised events are identical | the same test |
//! | a Deployment's burst: one row per pod up to the group threshold, one group row for the rest, each raised once; mute holds every new warning back, unmute resumes | `notify::a_burst_notifies_once_per_pod_and_once_for_the_rest_and_mute_holds_it_back` |
//!
//! Each test has its own `oxi-test-<rand>` namespace and asserts only about it; exact counts read
//! a feed narrowed to that namespace (`oxikube_testkit::ScopedEvents`), so the suites beside it on
//! the shared cluster cannot add rows. Every wait is bounded and prints the namespace's events
//! (as the service holds them and as `kubectl` lists them) when it runs out.

mod fallback;
mod notify;

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use oxikube_app::events::{
    EventFilter, EventService, EventsConfig, EventsState, EventsSubscription,
};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::store::Spawner;
use oxikube_domain::event::Event;
use oxikube_domain::ids::ClusterId;
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_ports::ClusterConnectorPort;
use oxikube_testkit::FakeClusterSourcePort;
use oxikube_testkit::integration::TestNamespace;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

/// Far longer than the kubelet takes to report a failed pull (well under a second on kind).
const DEADLINE: Duration = Duration::from_secs(60);

/// The budget from a `kubectl` watch seeing an event to the app holding it (epic E14, done-when 1).
const LIVE_WITHIN: Duration = Duration::from_secs(1);

fn tokio_spawner() -> Arc<dyn Spawner> {
    Arc::new(|task: BoxFuture<'static, ()>| {
        tokio::spawn(task);
    })
}

/// The real connector to the kind cluster.
fn kube_connector(kind: &Kind) -> KubeConnector {
    KubeConnector::new(
        kind.kubeconfig.clone(),
        PoolConfig::default(),
        ConnectorConfig::default(),
    )
}

/// One connected cluster and its `EventService`, as the app builds them, with the subscriber a
/// cluster tab holds (its warning badges): it keeps the cluster's one feed running whatever else
/// subscribes or leaves.
struct Chain {
    sessions: ClusterSessionManager,
    events: EventService,
    cluster: ClusterId,
    _tab: EventsSubscription,
}

impl Chain {
    /// Connects the kind context through `connector` and starts an `EventService` over it.
    async fn connect(kind: &Kind, connector: Arc<dyn ClusterConnectorPort>) -> Self {
        let entry = catalog_entry(&kind.context);
        let cluster = entry.cluster.clone();
        let sessions = ClusterSessionManager::with_config(
            connector,
            Arc::new(FakeClusterSourcePort::new()),
            Arc::new(TokioClock),
            SessionManagerConfig::default(),
        );
        sessions.open(&entry, SessionOptions::default());
        sessions.connect(&cluster).await.expect("connect");
        let events = EventService::new(
            sessions.clone(),
            Arc::new(TokioClock),
            EventsConfig {
                idle_grace: Duration::ZERO,
                ..EventsConfig::default()
            },
        );
        events.start(tokio_spawner());
        let tab = events.subscribe(&cluster, EventFilter::warnings());
        Self {
            sessions,
            events,
            cluster,
            _tab: tab,
        }
    }

    /// The events the service holds in `ns`, newest first.
    fn in_namespace(&self, ns: &TestNamespace) -> Vec<Arc<Event>> {
        self.events
            .query(&self.cluster, &EventFilter::new().namespaces([ns.name()]))
    }

    /// What the service holds in `ns`, and what `kubectl` lists there, for a failure message.
    fn diagnostics(&self, ns: &TestNamespace) -> String {
        let held = self
            .in_namespace(ns)
            .iter()
            .map(|e| {
                format!(
                    "{:?} {} {}/{} x{} {}",
                    e.event_type,
                    e.reason,
                    e.regarding.gvk.kind,
                    e.regarding.name,
                    e.count,
                    e.uid.as_deref().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "feed {:?}, status {:?}\nheld in {}:\n{held}\nkubectl:\n{}",
            self.events.state(&self.cluster),
            self.events.status(&self.cluster),
            ns.name(),
            ns.events()
        )
    }

    /// Waits until the feed has listed.
    async fn ready(&self, ns: &TestNamespace) {
        until(
            "the events feed to be ready",
            || self.diagnostics(ns),
            || self.events.state(&self.cluster) == EventsState::Ready,
        )
        .await;
    }
}

/// Polls `check` every 10 ms until it holds; fails after [`DEADLINE`] with `diagnostics`.
async fn until(what: &str, diagnostics: impl Fn() -> String, mut check: impl FnMut() -> bool) {
    let started = Instant::now();
    while !check() {
        assert!(
            started.elapsed() < DEADLINE,
            "timed out waiting for {what}\n{}",
            diagnostics()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
