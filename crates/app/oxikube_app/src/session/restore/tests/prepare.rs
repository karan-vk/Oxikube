//! `prepare`: reopen the saved clusters without connecting.

use oxikube_domain::session::NamespaceSelection;

use super::*;

#[test]
fn the_saved_clusters_reopen_in_tab_order_and_nothing_connects() {
    let h = Harness::new();
    h.save(&["c", "a", "b"], Some("a"));

    let plan = block_on(h.restorer.prepare()).expect("prepare");

    assert_eq!(h.open_names(), ["c", "a", "b"]);
    assert_eq!(plan.active, Some(id("a")));
    for name in ["a", "b", "c"] {
        assert_eq!(h.phase(name), SessionPhase::Disconnected, "{name}");
    }
    assert!(h.connects().is_empty(), "no connect before anyone asks");
}

#[test]
fn nothing_saved_reopens_nothing() {
    let h = Harness::new();
    let plan = block_on(h.restorer.prepare()).expect("prepare");
    assert!(plan.is_empty() && plan.dropped.is_empty());
    assert!(h.open_names().is_empty());
    assert_eq!(h.saved(), None, "restoring never invents a saved session");
}

#[test]
fn an_empty_saved_session_does_not_read_the_catalog() {
    let h = Harness::new();
    h.save(&[], None);
    block_on(h.restorer.prepare()).expect("prepare");
    assert!(h.source.recorded_calls().is_empty());
}

#[test]
fn a_cluster_no_kubeconfig_defines_is_dropped_and_forgotten() {
    let h = Harness::new();
    let gone = ctx("gone");
    let saved = SavedTabs::new(vec![id("a"), gone.cluster.clone(), id("b")], Some(id("b")))
        .with_titles([(gone.cluster, "Staging".to_owned())]);
    block_on(h.store.save(&saved)).unwrap();

    let plan = block_on(h.restorer.prepare()).expect("prepare");

    assert_eq!(h.open_names(), ["a", "b"], "only what exists reopens");
    assert_eq!(plan.dropped.len(), 1);
    assert_eq!(plan.dropped[0].label(), "Staging");
    let after = h.saved().expect("still saved");
    assert_eq!(after.open, [id("a"), id("b")]);
    assert_eq!(after.active, Some(id("b")));

    // The next launch finds nothing to report.
    let second = Harness::on_state(h.state, RestoreConfig::default());
    let plan = block_on(second.restorer.prepare()).expect("prepare");
    assert!(plan.dropped.is_empty());
}

#[test]
fn the_remembered_namespace_selection_is_applied_before_any_connect() {
    let first = Harness::new();
    first.sessions.open(&ctx("a"), Default::default());
    block_on(
        first
            .namespaces
            .select(&id("a"), NamespaceSelection::from_names(["prod", "dev"])),
    )
    .expect("select");
    first.save(&["a", "b"], Some("a"));

    // A new launch over the same state.
    let h = Harness::on_state(first.state, RestoreConfig::default());
    block_on(h.restorer.prepare()).expect("prepare");

    let session = h.sessions.get(&id("a")).expect("session");
    assert_eq!(
        session.namespace_selection(),
        &NamespaceSelection::from_names(["prod", "dev"])
    );
    assert_eq!(
        h.sessions.get(&id("b")).unwrap().namespace_selection(),
        &NamespaceSelection::All,
        "nothing remembered: the session keeps what it opened with"
    );
    assert!(
        h.connects().is_empty(),
        "the scope is set while still disconnected"
    );
}
