//! `LayoutStore` over the fake state port.

use std::sync::Arc;

use futures::executor::block_on;
use oxikube_domain::ErrorKind;
use oxikube_ports::{StateKey, StatePort, StateTable};
use oxikube_testkit::fakes::FakeStatePort;
use oxikube_ui::dock::DockAreaState;
use serde_json::json;

use super::store;
use crate::persistence::{
    LAYOUT_SCHEMA_VERSION, LAYOUT_TABLE, LayoutError, LayoutStore, LoadOutcome, MAIN_WINDOW_ID,
    SerializedWorkspace,
};

fn layout() -> SerializedWorkspace {
    SerializedWorkspace {
        version: LAYOUT_SCHEMA_VERSION,
        window: None,
        active_pane: Some(0),
        dock_area: DockAreaState::default(),
    }
}

#[test]
fn save_then_load_round_trips_and_clear_forgets() {
    let fake = Arc::new(FakeStatePort::new());
    let store = store(fake);
    block_on(async {
        assert!(matches!(store.load().await.unwrap(), LoadOutcome::Missing));
        store.save(&layout()).await.unwrap();
        match store.load().await.unwrap() {
            LoadOutcome::Loaded(read) => assert_eq!(read, layout()),
            other => panic!("{other:?}"),
        }
        assert!(store.clear().await.unwrap());
        assert!(!store.clear().await.unwrap());
        assert!(matches!(store.load().await.unwrap(), LoadOutcome::Missing));
    });
}

#[test]
fn each_window_has_its_own_row() {
    let fake = Arc::new(FakeStatePort::new());
    let main = LayoutStore::new(fake.clone(), "main").unwrap();
    let other = LayoutStore::new(fake.clone(), "window-2").unwrap();
    block_on(async {
        main.save(&layout()).await.unwrap();
        assert!(matches!(other.load().await.unwrap(), LoadOutcome::Missing));
        let table = StateTable::new(LAYOUT_TABLE).unwrap();
        let rows = fake.table_list(&table, None).await.unwrap();
        assert_eq!(
            rows.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            [MAIN_WINDOW_ID]
        );
    });
}

#[test]
fn an_unusable_stored_layout_is_discarded_not_an_error() {
    let fake = Arc::new(FakeStatePort::new());
    let store = store(fake.clone());
    let table = StateTable::new(LAYOUT_TABLE).unwrap();
    let key = StateKey::new(MAIN_WINDOW_ID).unwrap();
    block_on(async {
        for (stored, newer) in [
            (json!({ "version": 7, "whatever": true }), true),
            (json!("not a layout"), false),
            (json!({ "version": 1, "dock_area": "nope" }), false),
        ] {
            fake.table_put(&table, &key, stored).await.unwrap();
            match store.load().await.unwrap() {
                LoadOutcome::Discarded(LayoutError::Newer { .. }) if newer => {}
                LoadOutcome::Discarded(LayoutError::Malformed(_)) if !newer => {}
                other => panic!("{other:?}"),
            }
        }
    });
}

#[test]
fn a_port_error_surfaces_from_load() {
    let fake = Arc::new(FakeStatePort::new());
    fake.script()
        .table_get
        .push_err(oxikube_domain::OxiError::internal("disk on fire"));
    let err = block_on(store(fake).load()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Internal);
}

#[test]
fn a_window_id_must_be_a_valid_state_key() {
    let fake = Arc::new(FakeStatePort::new());
    assert!(LayoutStore::new(fake.clone(), "has space").is_err());
    assert!(LayoutStore::new(fake, "").is_err());
}
