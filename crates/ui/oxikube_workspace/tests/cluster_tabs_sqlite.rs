//! The open cluster tabs and each cluster's layout through the real SQLite store: they survive
//! closing and reopening the database. No window and no GPUI executor here: the store runs on
//! its own thread, which a `#[gpui::test]` must not wait on (the controller's own behaviour is
//! covered over the in-memory fake in `cluster_tab::tests`).

use std::sync::Arc;

use futures::executor::block_on;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_state_sqlite::SqliteState;
use oxikube_workspace::{
    cluster_tab::{ClusterTabsStore, SavedTabs, cluster_layout_key},
    persistence::{LayoutStore, LoadOutcome, MAIN_WINDOW_ID, SerializedWorkspace},
};

fn cluster(name: &str) -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new(name))
}

fn layout(active_pane: usize) -> SerializedWorkspace {
    let mut layout = SerializedWorkspace {
        version: oxikube_workspace::persistence::LAYOUT_SCHEMA_VERSION,
        window: None,
        active_pane: Some(active_pane),
        dock_area: Default::default(),
    };
    layout.dock_area.version = Some(1);
    layout
}

#[test]
fn open_tabs_and_each_clusters_layout_survive_reopening_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let tabs = SavedTabs::new(
        vec![cluster("prod"), cluster("staging"), cluster("dev")],
        Some(cluster("staging")),
    );

    block_on(async {
        let state = Arc::new(SqliteState::open(&path).await.unwrap());
        let store = ClusterTabsStore::new(state.clone(), MAIN_WINDOW_ID).unwrap();
        assert_eq!(store.load().await.unwrap(), None, "nothing saved yet");
        store.save(&tabs).await.unwrap();
        // The last save wins.
        store
            .save(&SavedTabs::new(vec![cluster("dev")], None))
            .await
            .unwrap();
        store.save(&tabs).await.unwrap();

        // Each cluster's layout is its own row, next to the window's.
        for (name, pane) in [("prod", 0), ("staging", 1)] {
            let layouts =
                LayoutStore::new(state.clone(), &cluster_layout_key(&cluster(name))).unwrap();
            layouts.save(&layout(pane)).await.unwrap();
        }
        // Another window has its own tabs.
        let other = ClusterTabsStore::new(state, "window-2").unwrap();
        assert_eq!(other.load().await.unwrap(), None);
    });

    // A new process: the file is reopened from scratch.
    block_on(async {
        let state = Arc::new(SqliteState::open(&path).await.unwrap());
        let store = ClusterTabsStore::new(state.clone(), MAIN_WINDOW_ID).unwrap();
        assert_eq!(store.load().await.unwrap(), Some(tabs.clone()));
        for (name, pane) in [("prod", 0), ("staging", 1)] {
            let layouts =
                LayoutStore::new(state.clone(), &cluster_layout_key(&cluster(name))).unwrap();
            match layouts.load().await.unwrap() {
                LoadOutcome::Loaded(read) => assert_eq!(read, layout(pane), "{name}"),
                other => panic!("{name}: {other:?}"),
            }
        }
        let never = LayoutStore::new(state, &cluster_layout_key(&cluster("dev"))).unwrap();
        assert!(matches!(never.load().await.unwrap(), LoadOutcome::Missing));
    });
}

#[test]
fn the_saved_row_holds_cluster_ids_and_nothing_else() {
    // Non-negotiable 5: no names, tokens or paths in the state db.
    let tabs = SavedTabs::new(vec![cluster("prod")], Some(cluster("prod")));
    let json = serde_json::to_value(&tabs).unwrap();
    let mut keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["active", "open", "version"]);
    assert_eq!(json["open"][0], cluster("prod").as_str());
}
