//! A muted cluster stays muted across a restart of the real SQLite state adapter (E14-S05), and
//! only the cluster ids are written.

use std::sync::Arc;

use oxikube_app::events::{EventService, EventsConfig};
use oxikube_app::notifications::{MUTED_KEY, NotificationService};
use oxikube_app::session::ClusterSessionManager;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{StateKey, StatePort};
use oxikube_state_sqlite::SqliteState;
use oxikube_testkit::{FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort};
use serde_json::json;

async fn open(dir: &tempfile::TempDir) -> Arc<SqliteState> {
    Arc::new(
        SqliteState::open(dir.path().join("state.db"))
            .await
            .expect("opens"),
    )
}

fn centre(state: Arc<SqliteState>) -> NotificationService {
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let events = EventService::new(sessions.clone(), clock.clone(), EventsConfig::default());
    NotificationService::new(events, sessions, state, clock)
}

fn prod() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("prod"))
}

#[tokio::test]
async fn a_muted_cluster_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let centre = centre(open(&dir).await);
        centre.load().await;
        assert!(centre.mute(&prod()).await.expect("saved"));
    }
    // A new process: the database is opened again.
    let state = open(&dir).await;
    let centre = centre(state.clone());
    assert!(!centre.is_muted(&prod()), "not before the load");
    centre.load().await;
    assert!(centre.is_muted(&prod()));
    let stored = state
        .kv_get(&StateKey::new(MUTED_KEY).unwrap())
        .await
        .unwrap()
        .expect("stored");
    assert_eq!(
        stored,
        json!({ "clusters": [prod()] }),
        "ids only, no message"
    );

    assert!(centre.unmute(&prod()).await.expect("saved"));
    let centre = self::centre(open(&dir).await);
    centre.load().await;
    assert!(!centre.is_muted(&prod()));
}
