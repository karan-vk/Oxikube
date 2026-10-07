//! A network outage end to end (E06-F440): the real liveness loop gives up, the session goes to
//! `Error`, and the manager reconnects by itself once the network is back.
//!
//! The outage is a local TCP proxy in front of the kind API server: the test's kubeconfig points
//! at the proxy, and "cutting the network" closes every proxied connection and refuses new ones.
//! The cluster itself is never touched, so suites running beside this one see nothing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::StreamExt as _;
use kube::config::Kubeconfig;
use oxikube_app::session::{
    ClusterSessionManager, RetryPolicy, SessionChange, SessionManagerConfig, SessionOptions,
};
use oxikube_domain::session::SessionPhase;
use oxikube_kube::health::LivenessConfig;
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::FakeClusterSourcePort;
use parking_lot::Mutex;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{AbortHandle, JoinHandle};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};
use crate::eventually;

/// A TCP proxy to the API server that can be cut and restored.
struct Proxy {
    port: u16,
    down: Arc<AtomicBool>,
    connections: Arc<Mutex<Vec<AbortHandle>>>,
    accept: JoinHandle<()>,
}

impl Proxy {
    async fn start(upstream: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind proxy");
        let port = listener.local_addr().expect("proxy address").port();
        let down = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(Vec::<AbortHandle>::new()));
        let accept = tokio::spawn({
            let down = down.clone();
            let connections = connections.clone();
            async move {
                while let Ok((mut inbound, _)) = listener.accept().await {
                    if down.load(Ordering::SeqCst) {
                        // The network is gone: the connection is closed at once.
                        drop(inbound);
                        continue;
                    }
                    let upstream = upstream.clone();
                    let task = tokio::spawn(async move {
                        if let Ok(mut outbound) = TcpStream::connect(&upstream).await {
                            let _ =
                                tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                        }
                    });
                    connections.lock().push(task.abort_handle());
                }
            }
        });
        Self {
            port,
            down,
            connections,
            accept,
        }
    }

    /// Closes every open connection and refuses new ones.
    fn cut(&self) {
        self.down.store(true, Ordering::SeqCst);
        for connection in self.connections.lock().drain(..) {
            connection.abort();
        }
    }

    fn restore(&self) {
        self.down.store(false, Ordering::SeqCst);
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.cut();
        self.accept.abort();
    }
}

/// The kind cluster's `server` entry in `kubeconfig`.
fn server(kubeconfig: &mut Kubeconfig) -> &mut Option<String> {
    &mut kubeconfig.clusters[0]
        .cluster
        .as_mut()
        .expect("kind cluster body")
        .server
}

#[tokio::test]
async fn a_network_outage_ends_in_error_and_the_session_reconnects_by_itself() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let mut kubeconfig = kind.kubeconfig.clone();
    let upstream = server(&mut kubeconfig)
        .as_deref()
        .expect("kind server URL")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .to_owned();
    let proxy = Proxy::start(upstream).await;
    // The kind serving certificate names 127.0.0.1, so TLS verification still holds.
    *server(&mut kubeconfig) = Some(format!("https://127.0.0.1:{}", proxy.port));

    // Probes every second, giving up after three failures: an outage is seen in about 3 s.
    let liveness = LivenessConfig {
        interval: Duration::from_secs(1),
        probe_timeout: Duration::from_secs(1),
        failure_threshold: 3,
        ..LivenessConfig::default()
    };
    let connector = KubeConnector::new(
        kubeconfig,
        PoolConfig::default(),
        ConnectorConfig {
            liveness,
            ..ConnectorConfig::default()
        },
    );
    let manager = ClusterSessionManager::with_config(
        Arc::new(connector),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(TokioClock),
        SessionManagerConfig {
            auto_reconnect: Some(RetryPolicy {
                max_delay: Duration::from_secs(2),
                ..RetryPolicy::auto_reconnect()
            }),
            ..SessionManagerConfig::default()
        },
    );
    let entry = catalog_entry(&kind.context);
    let cluster = entry.cluster.clone();
    manager.open(&entry, SessionOptions::default());
    let mut updates = manager.subscribe();
    let state = manager.connect(&cluster).await.expect("connect");
    assert_eq!(state.phase(), SessionPhase::Ready, "{state:?}");

    let diagnostics = || format!("{:?}", manager.get(&cluster).map(|s| s.state().clone()));
    proxy.cut();
    eventually("Error with a reconnect planned", diagnostics, || async {
        manager
            .get(&cluster)
            .is_some_and(|s| s.phase() == SessionPhase::Error && s.auto_reconnect().is_some())
    })
    .await;
    // The network stays down for a while: attempts fail and the schedule goes on.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let session = manager.get(&cluster).unwrap();
    assert!(!session.is_connected(), "{:?}", session.state());
    assert!(session.auto_reconnect().is_some(), "{:?}", session.state());

    proxy.restore();
    eventually("Ready again without a user action", diagnostics, || async {
        manager
            .get(&cluster)
            .is_some_and(|s| s.phase() == SessionPhase::Ready)
    })
    .await;
    let session = manager.get(&cluster).unwrap();
    assert_eq!(session.auto_reconnect(), None);
    assert!(session.resources().is_some(), "the ports are back");

    let mut phases = Vec::new();
    while let Some(Some(Ok(update))) = futures::FutureExt::now_or_never(updates.next()) {
        if let SessionChange::StateChanged { state, .. } = update.change {
            phases.push(state.phase());
        }
    }
    eprintln!("phases: {phases:?}");
    let first_error = phases
        .iter()
        .position(|p| *p == SessionPhase::Error)
        .expect("the outage reached Error");
    assert!(
        phases[..first_error].contains(&SessionPhase::Degraded),
        "Degraded before Error: {phases:?}"
    );
    assert_eq!(phases.last(), Some(&SessionPhase::Ready), "{phases:?}");
    assert!(
        phases[first_error..].contains(&SessionPhase::Connecting),
        "reconnected by itself: {phases:?}"
    );

    // The new connection's liveness loop runs: the session stays healthy.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(manager.get(&cluster).unwrap().phase(), SessionPhase::Ready);
}
