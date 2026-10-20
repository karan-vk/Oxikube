//! Port-forward favourites against a real SQLite file (E15-S06): they survive a restart next to
//! the data other features keep in the same database, and hold no secret.

use std::sync::Arc;

use oxikube_app::portforward::favourites::{FavouriteForward, FavouriteStore, TABLE};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::portforward::{ForwardPort, ForwardSpec};
use oxikube_ports::{StateKey, StatePort, StateTable};
use oxikube_state_sqlite::SqliteState;
use serde_json::json;

async fn open(dir: &tempfile::TempDir) -> Arc<SqliteState> {
    Arc::new(
        SqliteState::open(dir.path().join("state.db"))
            .await
            .expect("opens"),
    )
}

fn favourite(cluster: &ClusterId, name: &str, remote: u16) -> FavouriteForward {
    let target =
        ResourceRef::namespaced(cluster.clone(), Gvk::new("", "v1", "Service"), "shop", name);
    FavouriteForward::new(
        ForwardSpec::new(target, ForwardPort::Number(remote)).with_local_port(remote + 10_000),
        Some(format!("{name} tunnel")),
        1_700_000_000_000,
    )
}

#[tokio::test]
async fn favourites_survive_a_restart_beside_other_state() {
    let dir = tempfile::tempdir().unwrap();
    let a = ClusterId::new("kubeconfig", &ContextName::from("a"));
    let b = ClusterId::new("kubeconfig", &ContextName::from("b"));
    let (db, api) = (favourite(&a, "db", 5432), favourite(&b, "api", 80));
    {
        let state = open(&dir).await;
        // Data an older version of the app wrote: the database already exists and is in use.
        state
            .kv_set(
                &StateKey::new("recents.commands").unwrap(),
                json!(["pod::Delete"]),
            )
            .await
            .unwrap();
        let store = FavouriteStore::new(state);
        store.put(&db).await.unwrap();
        store.put(&api).await.unwrap();
    }
    // A new process: the migrations run again and change nothing.
    let state = open(&dir).await;
    assert!(
        state
            .kv_get(&StateKey::new("recents.commands").unwrap())
            .await
            .unwrap()
            .is_some()
    );
    let store = FavouriteStore::new(state);
    assert_eq!(store.list_for(&a).await.unwrap(), std::slice::from_ref(&db));
    assert_eq!(store.list().await.unwrap().len(), 2);
    assert!(store.remove(&db.id).await.unwrap());
    assert_eq!(store.list().await.unwrap(), [api]);
}

#[tokio::test]
async fn a_stored_row_has_no_secret_fields() {
    let dir = tempfile::tempdir().unwrap();
    let state = open(&dir).await;
    let cluster = ClusterId::new("kubeconfig", &ContextName::from("a"));
    FavouriteStore::new(state.clone())
        .put(&favourite(&cluster, "db", 5432))
        .await
        .unwrap();
    let rows = state
        .table_list(&StateTable::new(TABLE).unwrap(), None)
        .await
        .unwrap();
    let text = rows[0].1.to_string().to_lowercase();
    for word in [
        "token",
        "password",
        "secret",
        "credential",
        "kubeconfig_data",
        "cert",
    ] {
        assert!(!text.contains(word), "{word} in {text}");
    }
    let mut keys: Vec<_> = rows[0].1.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["auto_start", "created_ms", "id", "name", "spec"]);
}
