//! `LazyState`: the state database that opens in the background.
//!
//! Plain `#[test]`s: the SQLite adapter's thread would trip GPUI's deterministic scheduler.

use futures::executor::block_on;
use oxikube_domain::ErrorKind;
use oxikube_ports::{StateKey, StatePort, StateTable};
use serde_json::json;

use crate::startup::state_db::LazyState;

fn key(name: &str) -> StateKey {
    StateKey::new(name).expect("valid key")
}

#[test]
fn calls_wait_for_the_open_and_then_persist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");

    let state = LazyState::new(path.clone());
    assert!(!path.exists(), "nothing opens until the first poll");
    block_on(state.kv_set(&key("greeting"), json!("hello"))).unwrap();
    assert!(path.exists());
    assert_eq!(
        block_on(state.kv_get(&key("greeting"))).unwrap(),
        Some(json!("hello"))
    );
    let table = StateTable::new("layouts").expect("valid table");
    block_on(state.table_put(&table, &key("main"), json!({ "v": 1 }))).unwrap();
    drop(state);

    // A new handle on the same file sees the data: it is the real SQLite adapter underneath.
    let reopened = LazyState::new(path);
    assert_eq!(
        block_on(reopened.kv_get(&key("greeting"))).unwrap(),
        Some(json!("hello"))
    );
    assert_eq!(
        block_on(reopened.table_get(&table, &key("main"))).unwrap(),
        Some(json!({ "v": 1 }))
    );
}

#[test]
fn a_failed_open_is_every_calls_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    // The parent "directory" is a file, so the database cannot be created.
    let blocker = dir.path().join("file");
    std::fs::write(&blocker, "x").unwrap();
    let state = LazyState::new(blocker.join("state.db"));

    for _ in 0..2 {
        let err = block_on(state.kv_get(&key("k"))).expect_err("the database cannot open");
        assert_eq!(err.kind(), ErrorKind::Internal);
        assert!(
            err.message().starts_with("state database unavailable"),
            "{}",
            err.message()
        );
    }
    assert!(block_on(state.kv_set(&key("k"), json!(1))).is_err());
}

#[test]
fn clones_share_one_database() {
    let dir = tempfile::tempdir().unwrap();
    let a = LazyState::new(dir.path().join("state.db"));
    let b = a.clone();
    block_on(a.kv_set(&key("shared"), json!(7))).unwrap();
    assert_eq!(block_on(b.kv_get(&key("shared"))).unwrap(), Some(json!(7)));
}
