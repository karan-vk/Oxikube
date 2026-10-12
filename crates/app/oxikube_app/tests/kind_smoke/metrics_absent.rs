//! A cluster without metrics-server (E13-S13, tier 1, ADR 0011's "never swallow"): the real
//! adapter against the real API server's answer for an API group it does not serve, and the state
//! the tables and the overview read from it.
//!
//! The shared kind cluster runs metrics-server, and uninstalling it would break every other suite,
//! so the connection under test asks for `metrics.k8s.io` under a group no server serves (the
//! same rewrite as `oxikube_kube`'s `metrics_kind`): the 404 is the API server's own, exactly what
//! a cluster without metrics-server answers. Everything else on the connection is untouched.
//!
//! * `MetricsPort` says `Unavailable(NotInstalled)`, not an error and not an empty list.
//! * The `MetricsService` state the tables' badge reads is `metrics unavailable: metrics-server is
//!   not installed (no metrics.k8s.io API)`, and no zero usage reaches the columns.
//! * The overview's feed still counts nodes and pods, carries the reason for its usage bars and
//!   top lists, and has no usage figure rather than a zero.

use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::future::BoxFuture;
use http::Request;
use kube::client::{Body, ClientBuilder};
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Client, Config};
use oxikube_app::cluster_overview::{OverviewFeed, OverviewInput, TOP_N};
use oxikube_app::metrics::live::{MetricsKind, MetricsService, MetricsState};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::store::{ResourceStore, Spawner, StoreOptions, StorePorts, StoreRuntime};
use oxikube_domain::OxiResult;
use oxikube_domain::metrics::MissingReason;
use oxikube_domain::session::NamespaceSelection;
use oxikube_kube::{
    ConnectorConfig, KubeConnector, KubeDiscovery, KubeMetrics, KubeResources, PoolConfig,
};
use oxikube_ports::{ClusterConnection, ClusterConnectorPort, ConnectRequest, MetricsOutcome};
use oxikube_testkit::FakeClusterSourcePort;
use tower::{Layer, Service};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

const DEADLINE: Duration = Duration::from_secs(60);

/// Sends `metrics.k8s.io` requests to a group no API server serves; everything else unchanged.
#[derive(Clone)]
struct HideMetricsGroup<S> {
    inner: S,
}

struct HideMetricsGroupLayer;

impl<S> Layer<S> for HideMetricsGroupLayer {
    type Service = HideMetricsGroup<S>;
    fn layer(&self, inner: S) -> Self::Service {
        HideMetricsGroup { inner }
    }
}

impl<S: Service<Request<Body>>> Service<Request<Body>> for HideMetricsGroup<S> {
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let rewritten = req.uri().to_string().replace(
            "/apis/metrics.k8s.io/",
            "/apis/metrics.absent.oxikube.test/",
        );
        *req.uri_mut() = rewritten.parse().expect("a rewritten uri");
        self.inner.call(req)
    }
}

/// The real connector, whose connections read metrics through a client that cannot see
/// `metrics.k8s.io`.
struct WithoutMetricsServer {
    inner: KubeConnector,
    kubeconfig: Kubeconfig,
}

#[async_trait]
impl ClusterConnectorPort for WithoutMetricsServer {
    async fn connect(&self, request: ConnectRequest) -> OxiResult<ClusterConnection> {
        let (cluster, context) = (request.cluster.clone(), request.context.clone());
        let mut connection = self.inner.connect(request).await?;
        let options = KubeConfigOptions {
            context: Some(context.to_string()),
            ..KubeConfigOptions::default()
        };
        let config = Config::from_custom_kubeconfig(self.kubeconfig.clone(), &options)
            .await
            .expect("the kind client config");
        let client = ClientBuilder::try_from(config)
            .expect("a client builder")
            .with_layer(&HideMetricsGroupLayer)
            .build();
        connection.ports.metrics = Arc::new(KubeMetrics::new(client, cluster));
        Ok(connection)
    }
}

fn tokio_spawner() -> Arc<dyn Spawner> {
    Arc::new(|task: BoxFuture<'static, ()>| {
        tokio::spawn(task);
    })
}

async fn until(what: &str, check: impl Fn() -> bool) {
    let started = Instant::now();
    while !check() {
        assert!(started.elapsed() < DEADLINE, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn store_of(client: Client, kind: &Kind) -> ResourceStore {
    let reader = Arc::new(KubeResources::new(
        client.clone(),
        KubeDiscovery::new(client),
    ));
    ResourceStore::new(
        catalog_entry(&kind.context).cluster,
        StorePorts {
            resources: reader.clone(),
            tables: reader,
        },
        StoreRuntime {
            spawner: tokio_spawner(),
            clock: Arc::new(TokioClock),
            probe: None,
        },
        StoreOptions::default(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_metrics_server_the_port_tables_and_overview_say_not_installed() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let entry = catalog_entry(&kind.context);
    let cluster = entry.cluster.clone();
    let sessions = ClusterSessionManager::with_config(
        Arc::new(WithoutMetricsServer {
            inner: KubeConnector::new(
                kind.kubeconfig.clone(),
                PoolConfig::default(),
                ConnectorConfig::default(),
            ),
            kubeconfig: kind.kubeconfig.clone(),
        }),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(TokioClock),
        SessionManagerConfig::default(),
    );
    sessions.open(&entry, SessionOptions::default());
    sessions.connect(&cluster).await.expect("connect");

    // The port: absence is an outcome, not an error.
    let port = sessions
        .get(&cluster)
        .and_then(|session| session.metrics())
        .expect("the connected cluster's metrics port");
    let not_installed = MetricsOutcome::Unavailable(MissingReason::NotInstalled);
    assert_eq!(
        port.node_metrics(&cluster).await.expect("not an error"),
        not_installed
    );
    assert_eq!(
        port.pod_metrics(&cluster, &NamespaceSelection::All)
            .await
            .expect("not an error"),
        not_installed
    );

    // The tables' state: the reason, shown; no reading.
    let service = MetricsService::new(sessions.clone(), Arc::new(TokioClock));
    service.start(tokio_spawner());
    let mut window = service.visibility();
    window.show(Some(cluster.clone()));
    let expected = MetricsState::Unavailable(MissingReason::NotInstalled);

    // The overview: a cluster-wide feed of pods and nodes (through a store of the real cluster)
    // and the live metrics service, as the view starts it.
    let admin = kind.admin_client().await;
    let started = OverviewFeed::start(OverviewInput {
        cluster: cluster.clone(),
        store: store_of(admin, &kind),
        metrics: Some(service.clone()),
        top_n: TOP_N,
    });
    let driver = tokio::spawn(started.driver);
    let feed = started.feed;

    until("the verdict for nodes and pods", || {
        service.state(&cluster, MetricsKind::Nodes) == expected
            && service.state(&cluster, MetricsKind::Pods) == expected
    })
    .await;
    assert_eq!(
        expected.unavailable_text().as_deref(),
        Some("metrics unavailable: metrics-server is not installed (no metrics.k8s.io API)")
    );
    let usage = service.usage(&cluster);
    assert_eq!(usage.counts(), (0, 0), "no reading, so no zero either");
    assert_eq!(usage.state(MetricsKind::Pods), expected);

    until(
        "the overview to count the cluster and carry the reason",
        || {
            let s = feed.snapshot();
            s.nodes_feed.is_listed() && s.pods_feed.is_listed() && s.node_metrics == expected
        },
    )
    .await;
    let snapshot = feed.snapshot();
    assert!(snapshot.nodes.total() >= 1, "{:?}", snapshot.nodes);
    assert!(snapshot.phases.total() >= 1, "{:?}", snapshot.phases);
    assert_eq!(snapshot.pod_metrics, expected);
    assert_eq!(snapshot.usage, None, "no usage figure, never a zero");
    assert!(
        snapshot.top.pods_cpu.is_empty() && snapshot.top.namespaces_memory.is_empty(),
        "{:?}",
        snapshot.top
    );
    driver.abort();
}
