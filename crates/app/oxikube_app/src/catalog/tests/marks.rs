//! Favourites and last-used: stored through `StatePort`, tolerant of a bad state db.

use std::time::Duration;

use futures::FutureExt as _;
use oxikube_domain::OxiError;
use oxikube_ports::{StateKey, StatePort as _, StateTable};
use oxikube_testkit::StateCall;
use serde_json::json;

use super::{Harness, id};
use crate::catalog::CATALOG_TABLE;

fn row(h: &Harness, name: &str) -> Option<serde_json::Value> {
    let table = StateTable::new(CATALOG_TABLE).unwrap();
    let key = StateKey::new(id(name).as_str()).unwrap();
    h.state
        .table_get(&table, &key)
        .now_or_never()
        .expect("fake state does not wait")
        .expect("table_get")
}

fn toggle(h: &Harness, name: &str, favourite: Option<bool>) -> bool {
    h.catalog
        .set_favourite(&id(name), favourite)
        .now_or_never()
        .expect("fake state does not wait")
        .expect("set_favourite")
}

#[test]
fn a_favourite_is_stored_and_loaded_back() {
    let h = Harness::new();
    assert!(toggle(&h, "b", Some(true)));
    assert_eq!(row(&h, "b"), Some(json!({ "favourite": true })));
    assert!(h.entry("b").favourite);
    assert!(!h.entry("a").favourite);
}

#[test]
fn toggling_flips_and_an_explicit_value_sets() {
    let h = Harness::new();
    assert!(toggle(&h, "a", None), "off -> on");
    assert!(!toggle(&h, "a", None), "on -> off");
    assert!(!toggle(&h, "a", Some(false)), "off stays off");
    assert!(toggle(&h, "a", Some(true)));
    assert!(toggle(&h, "a", Some(true)), "on stays on");
}

#[test]
fn marking_used_stamps_the_clock_time_and_keeps_the_favourite() {
    let h = Harness::new();
    toggle(&h, "a", Some(true));
    h.clock.advance(Duration::from_secs(90));
    let stamped = h
        .catalog
        .mark_used(&id("a"))
        .now_or_never()
        .expect("no wait")
        .expect("mark_used");
    assert_eq!(stamped, h.now());
    let entry = h.entry("a");
    assert_eq!(entry.last_used, Some(stamped));
    assert!(entry.favourite, "stamping must not drop the favourite");

    // And the other way round: toggling keeps the stamp.
    toggle(&h, "a", Some(false));
    assert_eq!(h.entry("a").last_used, Some(stamped));
}

#[test]
fn marks_are_loaded_with_one_table_read() {
    let h = Harness::new();
    toggle(&h, "a", Some(true));
    h.state.clear_calls();
    h.load();
    let calls = h.state.recorded_calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(matches!(&calls[0], StateCall::TableList(t, None) if t.as_str() == CATALOG_TABLE));
}

#[test]
fn a_state_db_that_cannot_be_read_does_not_hide_the_clusters() {
    let h = Harness::new();
    h.state
        .script()
        .table_list
        .push_err(OxiError::internal("database is locked"));
    let entries = h.load();
    assert_eq!(entries.len(), 3);
    assert!(
        entries
            .iter()
            .all(|e| !e.favourite && e.last_used.is_none())
    );
}

#[test]
fn a_row_of_an_unexpected_shape_is_skipped_and_replaced_on_the_next_write() {
    let h = Harness::new();
    let table = StateTable::new(CATALOG_TABLE).unwrap();
    for (key, value) in [
        (id("a").as_str().to_owned(), json!("not an object")),
        ("not-a-cluster-id".to_owned(), json!({ "favourite": true })),
    ] {
        h.state
            .table_put(&table, &StateKey::new(key).unwrap(), value)
            .now_or_never()
            .unwrap()
            .unwrap();
    }
    assert!(!h.entry("a").favourite);
    assert!(
        toggle(&h, "a", None),
        "the bad row reads as empty and is replaced"
    );
    assert_eq!(row(&h, "a"), Some(json!({ "favourite": true })));
}

#[test]
fn a_failed_write_is_an_error_and_changes_nothing() {
    let h = Harness::new();
    h.state
        .script()
        .table_put
        .push_err(OxiError::internal("disk full"));
    let result = h
        .catalog
        .set_favourite(&id("a"), Some(true))
        .now_or_never()
        .expect("no wait");
    assert!(result.is_err());
    assert!(!h.entry("a").favourite);
}

#[test]
fn a_failed_read_does_not_overwrite_stored_marks() {
    let h = Harness::new();
    toggle(&h, "a", Some(true));
    h.state
        .script()
        .table_get
        .push_err(OxiError::internal("database is locked"));
    let result = h
        .catalog
        .mark_used(&id("a"))
        .now_or_never()
        .expect("no wait");
    assert!(result.is_err(), "a transient read failure must surface");
    assert_eq!(row(&h, "a"), Some(json!({ "favourite": true })));
}

#[test]
fn source_changes_reach_the_catalog_subscribers() {
    use futures::StreamExt as _;
    let h = Harness::new();
    let mut changes = h.catalog.changes();
    h.source.set_contexts([super::ctx("a"), super::ctx("d")]);
    let diff = changes
        .next()
        .now_or_never()
        .flatten()
        .expect("the diff was pushed");
    assert_eq!(diff.added.len(), 1);
    assert_eq!(diff.removed.len(), 2);
}
