//! A saved layout through the real SQLite store: a split workspace survives a close and reopen of
//! the database, and a corrupt database falls back to a fresh one without touching the settings
//! file. No window and no GPUI executor here: the store runs on its own thread, which a
//! `#[gpui::test]` must not wait on.

use std::sync::Arc;

use futures::executor::block_on;
use gpui::{Axis, px};
use oxikube_state_sqlite::SqliteState;
use oxikube_ui::dock::{DockAreaState, DockPlacement, DockState, PanelInfo, PanelState};
use oxikube_workspace::{
    item::ITEM_PANEL_NAME,
    persistence::{
        LAYOUT_SCHEMA_VERSION, LayoutStore, LoadOutcome, MAIN_WINDOW_ID, SerializedWindow,
        SerializedWorkspace, WindowMode,
    },
};
use serde_json::json;

fn item(kind: &str, state: serde_json::Value) -> PanelState {
    let mut leaf = PanelState::new(ITEM_PANEL_NAME);
    leaf.info = PanelInfo::panel(json!({ "kind": kind, "state": state }));
    leaf
}

fn tabs(active: usize, children: Vec<PanelState>) -> PanelState {
    let mut group = PanelState::new("TabPanel");
    group.info = PanelInfo::tabs(active);
    group.children = children;
    group
}

fn stack(axis: Axis, sizes: &[f32], children: Vec<PanelState>) -> PanelState {
    let mut node = PanelState::new("StackPanel");
    node.info = PanelInfo::stack(sizes.iter().map(|s| px(*s)).collect(), axis);
    node.children = children;
    node
}

/// `(pods+nodes | (logs / yaml))` with a left dock holding a sidebar panel.
fn split_workspace() -> SerializedWorkspace {
    let mut sidebar = PanelState::new("ClusterSidebar");
    sidebar.info = PanelInfo::panel(json!({ "key": "sidebar", "state": { "expanded": ["prod"] } }));
    SerializedWorkspace {
        version: LAYOUT_SCHEMA_VERSION,
        window: Some(SerializedWindow {
            mode: WindowMode::Windowed,
            x: 40.0,
            y: 30.0,
            width: 1280.0,
            height: 800.0,
        }),
        active_pane: Some(1),
        dock_area: DockAreaState {
            version: Some(1),
            center: stack(
                Axis::Horizontal,
                &[700., 500.],
                vec![
                    tabs(
                        1,
                        vec![
                            item("pods", json!({ "ns": "default" })),
                            item("nodes", json!(null)),
                        ],
                    ),
                    stack(
                        Axis::Vertical,
                        &[400., 300.],
                        vec![
                            tabs(0, vec![item("logs", json!({ "pod": "web-0" }))]),
                            tabs(0, vec![item("yaml", json!("apiVersion: v1"))]),
                        ],
                    ),
                ],
            ),
            left_dock: Some(DockState::new(
                tabs(0, vec![sidebar]),
                DockPlacement::Left,
                px(260.),
                true,
            )),
            right_dock: None,
            bottom_dock: None,
        },
    }
}

#[test]
fn a_split_workspace_survives_closing_and_reopening_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let layout = split_workspace();

    block_on(async {
        let state = Arc::new(SqliteState::open(&path).await.unwrap());
        let store = LayoutStore::new(state, MAIN_WINDOW_ID).unwrap();
        assert!(matches!(store.load().await.unwrap(), LoadOutcome::Missing));
        store.save(&layout).await.unwrap();
        // A newer save replaces the older.
        let mut newer = layout.clone();
        newer.active_pane = Some(0);
        store.save(&newer).await.unwrap();
        store.save(&layout).await.unwrap();
    });

    // A new process: the file is reopened from scratch.
    block_on(async {
        let state = Arc::new(SqliteState::open(&path).await.unwrap());
        assert!(state.recovery().is_none());
        let store = LayoutStore::new(state, MAIN_WINDOW_ID).unwrap();
        match store.load().await.unwrap() {
            LoadOutcome::Loaded(read) => assert_eq!(read, layout),
            other => panic!("{other:?}"),
        }
    });
}

#[test]
fn a_corrupt_database_falls_back_to_a_fresh_one_and_settings_stay_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let settings = dir.path().join("settings.json");
    let keymap = dir.path().join("keymap.json");
    std::fs::write(&settings, br#"{"theme":"dark"}"#).unwrap();
    std::fs::write(&keymap, b"[]").unwrap();
    // Random bytes where the database should be.
    let junk: Vec<u8> = (0..16_384u32)
        .map(|i| (i.wrapping_mul(2_246_822_519) >> 11) as u8)
        .collect();
    std::fs::write(&path, &junk).unwrap();

    block_on(async {
        let state = Arc::new(SqliteState::open(&path).await.unwrap());
        let moved = state.recovery().expect("recovered").moved_to.clone();
        assert!(moved.to_string_lossy().contains("state.db.corrupt-"));
        assert_eq!(std::fs::read(&moved).unwrap(), junk);

        // The app starts with the default layout, and saving works again.
        let store = LayoutStore::new(state, MAIN_WINDOW_ID).unwrap();
        assert!(matches!(store.load().await.unwrap(), LoadOutcome::Missing));
        store.save(&split_workspace()).await.unwrap();
        assert!(matches!(
            store.load().await.unwrap(),
            LoadOutcome::Loaded(_)
        ));
    });
    assert_eq!(std::fs::read(&settings).unwrap(), br#"{"theme":"dark"}"#);
    assert_eq!(std::fs::read(&keymap).unwrap(), b"[]");
}
