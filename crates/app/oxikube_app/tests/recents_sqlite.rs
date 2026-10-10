//! Recents and jump history against a real SQLite file (E11-S11): they survive a restart, stay
//! capped and deduplicated, and a damaged value starts empty.

use std::sync::Arc;

use oxikube_app::{JumpRecents, RECENTS_CAPACITY, RecentsStore, StateRecents};
use oxikube_domain::command::{self, CommandId};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{StateKey, StatePort};
use oxikube_state_sqlite::SqliteState;
use serde_json::json;

async fn open(dir: &tempfile::TempDir) -> Arc<SqliteState> {
    Arc::new(
        SqliteState::open(dir.path().join("state.db"))
            .await
            .expect("opens"),
    )
}

#[tokio::test]
async fn command_recents_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let state = open(&dir).await;
        let recents = StateRecents::new(state);
        recents.record(CommandId::POD_DELETE);
        recents.record(CommandId::VIEW_ZOOM_IN);
        recents.record(CommandId::POD_DELETE);
        recents.flush().await;
    }
    // A new process: the database is opened again (the migrations run again, and change nothing).
    let state = open(&dir).await;
    let recents = StateRecents::new(state);
    recents.load().await;
    assert_eq!(
        recents.recent(),
        [CommandId::POD_DELETE, CommandId::VIEW_ZOOM_IN]
    );
}

#[tokio::test]
async fn the_stored_list_is_capped_and_deduplicated() {
    let dir = tempfile::tempdir().unwrap();
    let ids: Vec<_> = command::COMMANDS.iter().map(|meta| meta.id).collect();
    assert!(ids.len() > RECENTS_CAPACITY);
    {
        let recents = StateRecents::new(open(&dir).await);
        for id in ids.iter().chain(ids.iter().take(10)) {
            recents.record(*id);
        }
        recents.flush().await;
    }
    let recents = StateRecents::new(open(&dir).await);
    recents.load().await;
    let kept = recents.recent();
    assert_eq!(kept.len(), RECENTS_CAPACITY);
    let unique: std::collections::HashSet<_> = kept.iter().collect();
    assert_eq!(unique.len(), kept.len());
    assert_eq!(kept[0], ids[9], "the last one touched is first");
}

#[tokio::test]
async fn a_damaged_value_starts_empty() {
    let dir = tempfile::tempdir().unwrap();
    let state = open(&dir).await;
    state
        .kv_set(
            &StateKey::new("recents.commands").unwrap(),
            json!({"ids": {"not": "a list"}}),
        )
        .await
        .unwrap();
    let recents = StateRecents::new(state);
    recents.load().await;
    assert!(recents.recent().is_empty());
}

#[tokio::test]
async fn clearing_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let recents = StateRecents::new(open(&dir).await);
        recents.record(CommandId::POD_DELETE);
        recents.flush().await;
        recents.clear();
        recents.flush().await;
    }
    let recents = StateRecents::new(open(&dir).await);
    recents.load().await;
    assert!(recents.recent().is_empty());
}

#[tokio::test]
async fn jump_history_survives_a_restart_per_cluster() {
    let dir = tempfile::tempdir().unwrap();
    let prod = ClusterId::new("/kubeconfig", &ContextName::new("prod"));
    let staging = ClusterId::new("/kubeconfig", &ContextName::new("staging"));
    {
        let history = JumpRecents::new(open(&dir).await);
        history.record(&prod, "deploy kube-system");
        history.record(&prod, "pod app=nginx");
        history.record(&prod, "deploy  kube-system");
        history.record(&staging, "ns");
        history.flush().await;
    }
    let history = JumpRecents::new(open(&dir).await);
    history.load(&prod).await;
    history.load(&staging).await;
    assert_eq!(
        history.recent(&prod),
        ["deploy kube-system", "pod app=nginx"]
    );
    assert_eq!(history.recent(&staging), ["ns"]);
}

#[tokio::test]
async fn only_ids_and_jump_text_reach_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let state = open(&dir).await;
    let prod = ClusterId::new("/kubeconfig", &ContextName::new("prod"));
    let recents = StateRecents::new(state.clone());
    let history = JumpRecents::new(state.clone());
    recents.record(CommandId::POD_DELETE);
    history.record(&prod, "pod app=nginx");
    assert!(!history.record(&prod, "password=hunter2"));
    recents.flush().await;
    history.flush().await;
    let all = state.kv_list("").await.unwrap();
    let keys: Vec<_> = all
        .iter()
        .map(|(key, _)| key.as_str().to_string())
        .collect();
    assert!(keys.contains(&"recents.commands".to_string()), "{keys:?}");
    assert!(keys.contains(&format!("history.jump/{prod}")), "{keys:?}");
    let dump = serde_json::to_string(&all.iter().map(|(_, v)| v).collect::<Vec<_>>()).unwrap();
    assert!(!dump.contains("hunter2"), "{dump}");
}
