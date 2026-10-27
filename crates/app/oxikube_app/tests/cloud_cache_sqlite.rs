//! The cloud discovery cache against a real SQLite file (E18-S05): a listing survives a restart,
//! answers within the TTL without running the CLI, and the database holds no credential.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use jiff::Timestamp;
use oxikube_app::cloud::{CloudDiscoveryEvent, CloudDiscoveryService, DiscoveryDeps, RefreshMode};
use oxikube_app::session::ClusterSessionManager;
use oxikube_app::sources::{KubeconfigSourcesService, MemorySourceList};
use oxikube_app::store::Spawner;
use oxikube_ports::{
    CloudProvider, CloudScope, DiscoveredCluster, StatePort, StateTable, UserSource,
};
use oxikube_state_sqlite::SqliteState;
use oxikube_testkit::{
    FakeClockPort, FakeCloudDiscoveryPort, FakeClusterConnectorPort, FakeClusterSourcePort,
    FakeFsPort,
};

async fn open(dir: &tempfile::TempDir) -> Arc<SqliteState> {
    Arc::new(
        SqliteState::open(dir.path().join("state.db"))
            .await
            .expect("opens"),
    )
}

fn service(
    state: Arc<SqliteState>,
    clock: Arc<FakeClockPort>,
    port: Arc<FakeCloudDiscoveryPort>,
) -> CloudDiscoveryService {
    let source = Arc::new(FakeClusterSourcePort::new());
    let fs = Arc::new(FakeFsPort::new());
    let sources = KubeconfigSourcesService::new(
        source.clone(),
        fs.clone(),
        Arc::new(MemorySourceList::new([UserSource::default_source()])),
        PathBuf::from("/config/kubeconfigs"),
    );
    let spawner: Arc<dyn Spawner> = Arc::new(|task| {
        tokio::spawn(task);
    });
    CloudDiscoveryService::new(DiscoveryDeps {
        providers: vec![port],
        state,
        fs,
        sources,
        source: source.clone(),
        sessions: ClusterSessionManager::new(
            Arc::new(FakeClusterConnectorPort::new()),
            source,
            clock.clone(),
        ),
        clock,
        spawner,
    })
}

async fn run(service: &CloudDiscoveryService) -> Vec<CloudDiscoveryEvent> {
    let events = service.refresh(CloudProvider::Gcp, None, RefreshMode::IfStale);
    tokio::time::timeout(Duration::from_secs(10), events.collect())
        .await
        .expect("the run ends")
}

#[tokio::test]
async fn a_listing_survives_a_restart_and_answers_within_the_ttl() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(FakeClockPort::new(
        Timestamp::from_second(1_800_000_000).unwrap(),
    ));
    let project = CloudScope::Gcp {
        project: "shop".into(),
    };
    let cluster = DiscoveredCluster {
        endpoint: Some("https://203.0.113.10".into()),
        ca_data: Some("LS0tLS1CRUdJTiBDRVJUSUZJQ0FURS0tLS0t".into()),
        kubernetes_version: Some("1.30.2-gke.100".into()),
        ..DiscoveredCluster::new(project.clone(), "web", "europe-west1")
    };
    let port = || {
        Arc::new(
            FakeCloudDiscoveryPort::ready(CloudProvider::Gcp)
                .with_scopes([project.clone()])
                .with_clusters([cluster.clone()]),
        )
    };
    {
        let first = service(open(&dir).await, clock.clone(), port());
        let events = run(&first).await;
        assert!(matches!(
            events.last(),
            Some(CloudDiscoveryEvent::Finished { .. })
        ));
    }

    // A new process over the same database, a minute later.
    clock.advance(Duration::from_secs(60));
    let state = open(&dir).await;
    let again = port();
    let restarted = service(state.clone(), clock, again.clone());
    assert_eq!(
        restarted.cached(CloudProvider::Gcp).await,
        std::slice::from_ref(&cluster)
    );
    let events = run(&restarted).await;
    let [
        CloudDiscoveryEvent::Cached {
            clusters,
            stale: false,
            ..
        },
        CloudDiscoveryEvent::Finished { .. },
    ] = &events[..]
    else {
        panic!("answered from the cache: {events:?}");
    };
    assert_eq!(*clusters, [cluster]);
    assert!(again.recorded_calls().is_empty(), "no CLI ran");

    // What is stored is cluster data only.
    let rows = state
        .table_list(&StateTable::new("cloud_scopes").unwrap(), None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let text = rows[0].1.to_string();
    assert!(text.contains("203.0.113.10"), "{text}");
    for secret in ["token", "password", "client-key"] {
        assert!(!text.contains(secret), "no {secret} in {text}");
    }
}
