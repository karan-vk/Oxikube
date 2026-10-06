//! [`ColumnPrefsStore`]: the state key, a round trip through the `StatePort`, bad rows.

use std::sync::Arc;

use futures::executor::block_on;
use oxikube_domain::ids::Gvk;
use oxikube_ports::{StateKey, StatePort as _};
use oxikube_testkit::FakeStatePort;
use serde_json::json;

use crate::table::{ColumnPrefs, ColumnPrefsStore, PREFS_VERSION, SavedSort, prefs_key};

#[test]
fn keys_name_the_group_and_kind_not_the_version() {
    let key = |g, v, k| prefs_key(&Gvk::new(g, v, k)).unwrap().as_str().to_owned();
    assert_eq!(key("", "v1", "Pod"), "table.columns.core/Pod");
    assert_eq!(
        key("apps", "v1", "Deployment"),
        "table.columns.apps/Deployment"
    );
    assert_eq!(
        key("autoscaling", "v1", "HorizontalPodAutoscaler"),
        key("autoscaling", "v2", "HorizontalPodAutoscaler")
    );
}

#[test]
fn prefs_round_trip_and_bad_rows_read_as_none() {
    let state = Arc::new(FakeStatePort::new());
    let gvk = Gvk::new("", "v1", "Pod");
    let store = ColumnPrefsStore::new(state.clone(), &gvk).unwrap();
    assert_eq!(block_on(store.load()).unwrap(), None);

    let prefs = ColumnPrefs {
        order: vec!["status".into()],
        sort: Some(SavedSort {
            column: "age".into(),
            descending: false,
        }),
        ..ColumnPrefs::default()
    };
    block_on(store.save(&prefs)).unwrap();
    let loaded = block_on(store.load()).unwrap().unwrap();
    assert_eq!(loaded.version, PREFS_VERSION);
    assert_eq!(loaded.order, prefs.order);
    assert_eq!(loaded.sort, prefs.sort);

    let key = StateKey::new("table.columns.core/Pod").unwrap();
    block_on(state.kv_set(&key, json!({ "version": PREFS_VERSION + 1 }))).unwrap();
    assert_eq!(block_on(store.load()).unwrap(), None, "a newer build's row");
    block_on(state.kv_set(&key, json!({ "order": 7 }))).unwrap();
    assert_eq!(
        block_on(store.load()).unwrap(),
        None,
        "a row of another shape"
    );
}
