//! The negative check: a context whose token the API server rejects reaches `AuthRequired`
//! (with the reason, and a retry that ends the same way), not a hang or a blank session.

use std::sync::Arc;
use std::time::{Duration, Instant};

use oxikube_app::session::{ClusterSessionManager, SessionOptions};
use oxikube_domain::ids::ContextName;
use oxikube_domain::session::SessionPhase;
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::FakeClusterSourcePort;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

/// Far longer than a 401 takes; a hang fails here instead of stalling CI.
const TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::test]
async fn a_rejected_token_reaches_auth_required_and_retries_the_same_way() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    const BAD: &str = "oxi-smoke-bad-token";
    let mut kubeconfig = kind.kubeconfig.clone();
    // Not a credential: any string the API server cannot authenticate.
    kind.add_token_context(&mut kubeconfig, BAD, "not-a-valid-token");
    let connector = KubeConnector::new(
        kubeconfig,
        PoolConfig::default(),
        ConnectorConfig::default(),
    );
    let manager = ClusterSessionManager::new(
        Arc::new(connector),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(TokioClock),
    );
    let entry = catalog_entry(&ContextName::new(BAD));
    manager.open(&entry, SessionOptions::default());

    for attempt in 1..=2 {
        let started = Instant::now();
        let state = tokio::time::timeout(TIMEOUT, manager.connect(&entry.cluster))
            .await
            .unwrap_or_else(|_| panic!("attempt {attempt}: connect hung past {TIMEOUT:?}"))
            .expect("connect");
        eprintln!(
            "bad token, attempt {attempt}: {:?} after {:?}",
            state.phase(),
            started.elapsed()
        );
        assert_eq!(state.phase(), SessionPhase::AuthRequired, "{state:?}");
        let session = manager.get(&entry.cluster).expect("session");
        assert!(!session.is_connected());
        assert!(
            session.resources().is_none(),
            "no ports without a connection"
        );
        let reason = format!("{:?}", session.state());
        assert!(
            !reason.contains("not-a-valid-token"),
            "token leaked: {reason}"
        );
    }
}
