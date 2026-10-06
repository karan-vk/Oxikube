//! Remembered per cluster in `StatePort`: selection, favourites, typed names.

use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::StatePort;
use oxikube_testkit::StateCall;
use serde_json::json;

use super::*;
use crate::session::namespaces::{NamespacePrefs, prefs_key};

fn set(names: &[&str]) -> NamespaceSelection {
    NamespaceSelection::from_names(names)
}

#[test]
fn the_selection_is_remembered_and_restored_on_connect() {
    let h = Harness::new();
    h.connect("a", &["dev", "prod"]);
    h.run(h.service.select(&id("a"), set(&["prod"]))).unwrap();

    // A new run: a fresh manager session starts with All, the stored record says prod.
    let restarted = h.restarted();
    h.manager.close(&id("a"));
    h.manager.open(&ctx("a"), Default::default());
    assert_eq!(h.selection("a"), NamespaceSelection::All);

    let prefs = h.run(restarted.restore(&id("a"))).unwrap();

    assert_eq!(prefs.selection, set(&["prod"]));
    assert_eq!(h.selection("a"), set(&["prod"]));
}

#[test]
fn restore_sends_a_change_only_when_the_session_differs() {
    let mut h = Harness::new();
    h.run(h.service.restore(&id("a"))).unwrap();
    assert!(
        h.namespace_changes().is_empty(),
        "nothing stored, nothing changes"
    );

    h.run(h.service.select(&id("a"), set(&["x"]))).unwrap();
    h.namespace_changes();
    h.run(h.service.restore(&id("a"))).unwrap();
    assert!(h.namespace_changes().is_empty(), "session already has it");
}

#[test]
fn selection_and_favourites_are_stored_under_one_key_per_cluster() {
    let h = Harness::new();
    h.connect("a", &["x"]);
    h.connect("b", &["y"]);
    h.run(h.service.select(&id("a"), set(&["x"]))).unwrap();
    h.run(h.service.toggle_favourite(&id("a"), "x")).unwrap();
    h.run(h.service.select(&id("b"), set(&["y"]))).unwrap();

    let stored = |name: &str| {
        let key = prefs_key(&id(name));
        assert_eq!(key.as_str(), format!("cluster/{}/namespaces", id(name)));
        let value = h.run(h.state.kv_get(&key)).unwrap().expect("stored");
        serde_json::from_value::<NamespacePrefs>(value).unwrap()
    };
    assert_eq!(stored("a").selection, set(&["x"]));
    assert_eq!(stored("a").favourites.iter().collect::<Vec<_>>(), ["x"]);
    assert_eq!(stored("b").selection, set(&["y"]));
    assert!(stored("b").favourites.is_empty());
    // Only local state keys are written, never anything else.
    for call in h.state.recorded_calls() {
        if let StateCall::KvSet(key, _) = call {
            assert!(key.as_str().starts_with("cluster/"), "{key:?}");
        }
    }
}

#[test]
fn favourites_survive_a_restart_and_keep_their_order() {
    let h = Harness::new();
    h.run(h.service.toggle_favourite(&id("a"), "prod")).unwrap();
    h.run(h.service.toggle_favourite(&id("a"), "dev")).unwrap();
    h.run(h.service.toggle_favourite(&id("a"), "stage"))
        .unwrap();
    let outcome = h.run(h.service.toggle_favourite(&id("a"), "dev")).unwrap();
    assert!(!outcome.prefs.favourites.contains("dev"), "toggled off");

    let prefs = h.run(h.restarted().prefs(&id("a"))).unwrap();

    assert_eq!(
        prefs.favourites.iter().collect::<Vec<_>>(),
        ["prod", "stage"]
    );
}

#[test]
fn a_selection_change_does_not_touch_the_favourites() {
    let h = Harness::new();
    h.connect("a", &["x", "y"]);
    h.run(h.service.toggle_favourite(&id("a"), "x")).unwrap();
    h.run(h.service.select(&id("a"), set(&["y"]))).unwrap();

    let prefs = h.run(h.service.prefs(&id("a"))).unwrap();
    assert!(prefs.favourites.contains("x"));
}

#[test]
fn an_unreadable_record_is_ignored() {
    let h = Harness::new();
    h.run(
        h.state
            .kv_set(&prefs_key(&id("a")), json!({"selection": 42})),
    )
    .unwrap();

    let prefs = h.run(h.service.prefs(&id("a"))).unwrap();

    assert_eq!(prefs, NamespacePrefs::default());
}

#[test]
fn a_record_from_an_older_build_still_loads() {
    let h = Harness::new();
    h.run(
        h.state
            .kv_set(&prefs_key(&id("a")), json!({"selection": {"set": ["x"]}})),
    )
    .unwrap();

    let prefs = h.run(h.service.prefs(&id("a"))).unwrap();

    assert_eq!(prefs.selection, set(&["x"]));
    assert!(prefs.favourites.is_empty());
}

#[test]
fn a_failed_write_is_an_error_and_the_session_still_changed() {
    let h = Harness::new();
    h.connect("a", &["x"]);
    h.state
        .script()
        .kv_set
        .push_err(oxikube_domain::OxiError::internal("disk full"));

    let err = h
        .run(h.service.select(&id("a"), set(&["x"])))
        .expect_err("write failed");

    assert_eq!(err.kind(), oxikube_domain::ErrorKind::Internal);
    assert_eq!(h.selection("a"), set(&["x"]), "the view still changed");
    // The next change stores everything that is pending.
    h.run(h.service.toggle_favourite(&id("a"), "x")).unwrap();
    let stored = h.run(h.restarted().prefs(&id("a"))).unwrap();
    assert_eq!(stored.selection, set(&["x"]));
}

/// Reopens cluster `a` the way the catalog does, with these settings: a session that starts
/// at the cluster's default namespace.
fn reopen_with_prefs(h: &Harness, prefs: oxikube_ports::ClusterPrefs) {
    h.manager.close(&id("a"));
    h.manager.set_prefs_table(
        oxikube_ports::ClusterPrefsTable::new(oxikube_ports::ClusterPrefs::default())
            .with_cluster(id("a"), prefs),
    );
    h.manager.open_configured(&ctx("a"));
}

fn payments_prefs() -> oxikube_ports::ClusterPrefs {
    oxikube_ports::ClusterPrefs {
        default_namespace: Some("payments".into()),
        ..Default::default()
    }
}

#[test]
fn restore_keeps_the_default_namespace_when_nothing_is_remembered() {
    let mut h = Harness::new();
    reopen_with_prefs(&h, payments_prefs());
    assert_eq!(h.selection("a"), set(&["payments"]));
    h.namespace_changes();

    let prefs = h.run(h.service.restore(&id("a"))).unwrap();

    assert_eq!(
        h.selection("a"),
        set(&["payments"]),
        "not overwritten by All"
    );
    assert_eq!(
        prefs.selection,
        set(&["payments"]),
        "the prefs say what the session has"
    );
    assert!(h.namespace_changes().is_empty());
    let reconciled = h.run(h.service.start(&id("a"))).unwrap();
    assert_eq!(reconciled.prefs.selection, set(&["payments"]));
}

#[test]
fn a_remembered_selection_beats_the_default_namespace() {
    let h = Harness::new();
    h.run(h.service.select(&id("a"), set(&["prod"]))).unwrap();
    reopen_with_prefs(&h, payments_prefs());
    assert_eq!(h.selection("a"), set(&["payments"]));

    h.run(h.restarted().restore(&id("a"))).unwrap();

    assert_eq!(h.selection("a"), set(&["prod"]));
}

#[test]
fn a_session_reopened_with_a_new_default_namespace_follows_it_until_the_user_picks() {
    let h = Harness::new();
    h.run(h.service.restore(&id("a"))).unwrap();
    reopen_with_prefs(&h, payments_prefs());

    let prefs = h.run(h.service.restore(&id("a"))).unwrap();

    assert_eq!(prefs.selection, set(&["payments"]));
    assert_eq!(h.selection("a"), set(&["payments"]));
}

#[test]
fn a_favourite_pinned_before_restore_does_not_replace_the_default_namespace() {
    let h = Harness::new();
    reopen_with_prefs(&h, payments_prefs());
    h.run(h.service.toggle_favourite(&id("a"), "dev")).unwrap();

    h.run(h.restarted().restore(&id("a"))).unwrap();

    assert_eq!(h.selection("a"), set(&["payments"]));
}
