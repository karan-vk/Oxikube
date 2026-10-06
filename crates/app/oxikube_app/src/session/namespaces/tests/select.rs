//! Selecting namespaces: one event per change, the empty-set rule, the scope delta.

use oxikube_domain::ids::Scope;
use oxikube_domain::session::{NamespaceSelection, WatchScope};

use super::*;
use crate::session::namespaces::{ClusterWide, ScopeDelta};

fn set(names: &[&str]) -> NamespaceSelection {
    NamespaceSelection::from_names(names)
}

#[test]
fn switching_ab_to_bc_sends_one_change_with_the_new_scope() {
    let mut h = Harness::new();
    h.connect("a", &["a", "b", "c"]);
    h.run(h.service.select(&id("a"), set(&["a", "b"]))).unwrap();
    h.namespace_changes();

    let outcome = h.run(h.service.select(&id("a"), set(&["b", "c"]))).unwrap();

    assert!(outcome.changed);
    assert_eq!(h.namespace_changes(), vec![(id("a"), set(&["b", "c"]))]);
    let session = h.manager.get(&id("a")).unwrap();
    assert_eq!(
        session.watch_scope(Scope::Namespaced),
        WatchScope::Namespaces(vec!["b".into(), "c".into()])
    );
}

#[test]
fn selecting_the_same_set_again_sends_nothing() {
    let mut h = Harness::new();
    h.connect("a", &["a", "b"]);
    h.run(h.service.select(&id("a"), set(&["a"]))).unwrap();
    h.namespace_changes();

    let outcome = h.run(h.service.select(&id("a"), set(&["a"]))).unwrap();

    assert!(!outcome.changed);
    assert!(h.namespace_changes().is_empty());
}

#[test]
fn an_empty_selection_means_all() {
    let mut h = Harness::new();
    h.connect("a", &["a"]);
    h.run(h.service.select(&id("a"), set(&["a"]))).unwrap();
    h.namespace_changes();

    // Unticking the last namespace.
    let mut sel = h.selection("a");
    sel.remove("a");
    let outcome = h.run(h.service.select(&id("a"), sel)).unwrap();

    assert_eq!(h.selection("a"), NamespaceSelection::All);
    assert_eq!(outcome.prefs.selection, NamespaceSelection::All);
    assert_eq!(
        h.namespace_changes(),
        vec![(id("a"), NamespaceSelection::All)]
    );
}

#[test]
fn selecting_on_a_session_that_is_not_open_is_not_found() {
    let h = Harness::new();
    let unknown = id("zzz");
    let err = h
        .run(h.service.select(&unknown, set(&["a"])))
        .expect_err("no such session");
    assert_eq!(err.kind(), oxikube_domain::ErrorKind::NotFound);
    // Nothing was remembered for it.
    assert!(
        h.state
            .recorded_calls()
            .iter()
            .all(|c| !matches!(c, oxikube_testkit::StateCall::KvSet(..)))
    );
}

#[test]
fn each_cluster_has_its_own_selection() {
    let mut h = Harness::new();
    h.connect("a", &["x"]);
    h.connect("b", &["y"]);
    h.run(h.service.select(&id("a"), set(&["x"]))).unwrap();
    h.run(h.service.select(&id("b"), set(&["y"]))).unwrap();

    assert_eq!(h.selection("a"), set(&["x"]));
    assert_eq!(h.selection("b"), set(&["y"]));
    assert_eq!(
        h.run(h.service.prefs(&id("a"))).unwrap().selection,
        set(&["x"])
    );
    assert_eq!(
        h.namespace_changes(),
        vec![(id("a"), set(&["x"])), (id("b"), set(&["y"]))]
    );
}

#[test]
fn the_scope_delta_keeps_the_namespaces_that_stay() {
    let ab = WatchScope::Namespaces(vec!["a".into(), "b".into()]);
    let bc = WatchScope::Namespaces(vec!["b".into(), "c".into()]);

    let delta = ScopeDelta::between(&ab, &bc);

    assert_eq!(delta.started, ["c"]);
    assert_eq!(delta.stopped, ["a"]);
    assert_eq!(delta.kept, ["b"]);
    assert_eq!(delta.cluster_wide, ClusterWide::Unchanged);
    assert!(ScopeDelta::between(&bc, &bc).is_empty());
}

#[test]
fn the_scope_delta_switches_between_cluster_wide_and_namespaced() {
    let ab = WatchScope::Namespaces(vec!["a".into(), "b".into()]);

    let narrow = ScopeDelta::between(&WatchScope::Cluster, &ab);
    assert_eq!(narrow.cluster_wide, ClusterWide::Stopped);
    assert_eq!(narrow.started, ["a", "b"]);

    let widen = ScopeDelta::between(&ab, &WatchScope::Cluster);
    assert_eq!(widen.cluster_wide, ClusterWide::Started);
    assert_eq!(widen.stopped, ["a", "b"]);
    assert!(widen.kept.is_empty());
    assert!(ScopeDelta::between(&WatchScope::Cluster, &WatchScope::Cluster).is_empty());
}
