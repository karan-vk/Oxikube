//! The namespace list, the stale-name drop and the 403 fallback.

use oxikube_domain::OxiError;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::{ListPage, StatePort};
use oxikube_testkit::StateCall;

use super::*;
use crate::session::namespaces::{
    NamespacePrefs, NamespaceSource, is_valid_namespace_name, prefs_key,
};

fn set(names: &[&str]) -> NamespaceSelection {
    NamespaceSelection::from_names(names)
}

#[test]
fn the_catalog_is_the_clusters_sorted_namespaces() {
    let h = Harness::new();
    h.connect("a", &["prod", "default", "dev"]);

    let catalog = h.run(h.service.catalog(&id("a"))).unwrap();

    assert_eq!(catalog.source, NamespaceSource::Cluster);
    assert_eq!(catalog.names, ["default", "dev", "prod"]);
    assert!(catalog.contains("dev") && !catalog.contains("nope"));
}

#[test]
fn a_long_namespace_list_is_read_page_by_page() {
    let h = Harness::new();
    h.connect("a", &[]);
    let ports = h.connector.ports_for(&id("a"));
    let page = |names: &[&str], next: Option<&str>| {
        let mut page = ListPage::complete(
            names
                .iter()
                .map(|n| oxikube_domain::ObjectMeta::named(*n))
                .collect(),
        );
        page.continue_token = next.map(str::to_owned);
        page
    };
    ports
        .resources
        .script()
        .list_metadata
        .push_ok(page(&["a", "b"], Some("t1")));
    ports
        .resources
        .script()
        .list_metadata
        .push_ok(page(&["c"], None));

    let catalog = h.run(h.service.catalog(&id("a"))).unwrap();

    assert_eq!(catalog.names, ["a", "b", "c"]);
}

#[test]
fn a_stored_namespace_that_no_longer_exists_is_dropped() {
    let mut h = Harness::new();
    h.connect("a", &["dev", "prod"]);
    h.run(h.service.select(&id("a"), set(&["prod", "gone"])))
        .unwrap();
    h.namespace_changes();

    let reconciled = h.run(h.service.reconcile(&id("a"))).unwrap();

    assert_eq!(reconciled.dropped, ["gone"]);
    assert_eq!(h.selection("a"), set(&["prod"]));
    assert_eq!(reconciled.prefs.selection, set(&["prod"]));
    assert_eq!(h.namespace_changes(), vec![(id("a"), set(&["prod"]))]);
    let stored = h.run(h.restarted().prefs(&id("a"))).unwrap();
    assert_eq!(stored.selection, set(&["prod"]), "the drop is remembered");
}

#[test]
fn dropping_every_stale_namespace_falls_back_to_all() {
    let h = Harness::new();
    h.connect("a", &["dev"]);
    h.run(h.service.select(&id("a"), set(&["gone", "also-gone"])))
        .unwrap();

    let reconciled = h.run(h.service.reconcile(&id("a"))).unwrap();

    assert_eq!(reconciled.dropped, ["also-gone", "gone"]);
    assert_eq!(h.selection("a"), NamespaceSelection::All);
}

#[test]
fn start_restores_then_drops_the_stale_ones() {
    let h = Harness::new();
    h.run(
        h.state.kv_set(
            &prefs_key(&id("a")),
            serde_json::to_value(NamespacePrefs {
                selection: set(&["dev", "gone"]),
                ..Default::default()
            })
            .unwrap(),
        ),
    )
    .unwrap();
    h.connect("a", &["dev", "prod"]);

    let reconciled = h.run(h.service.start(&id("a"))).unwrap();

    assert_eq!(reconciled.dropped, ["gone"]);
    assert_eq!(h.selection("a"), set(&["dev"]));
}

#[test]
fn a_forbidden_list_falls_back_to_the_typed_names_and_prunes_nothing() {
    let h = Harness::new();
    h.connect("a", &["dev"]);
    let ports = h.connector.ports_for(&id("a"));
    h.run(h.service.add_typed(&id("a"), "team-a")).unwrap();
    h.run(h.service.select(&id("a"), set(&["team-a"]))).unwrap();
    ports
        .resources
        .script()
        .list_metadata
        .push_err(OxiError::forbidden("namespaces is forbidden"));
    ports
        .resources
        .script()
        .list_metadata
        .push_err(OxiError::forbidden("namespaces is forbidden"));

    let catalog = h.run(h.service.catalog(&id("a"))).unwrap();
    assert_eq!(catalog.source, NamespaceSource::Forbidden);
    assert_eq!(catalog.names, ["team-a"]);

    let reconciled = h.run(h.service.reconcile(&id("a"))).unwrap();
    assert!(reconciled.dropped.is_empty());
    assert_eq!(h.selection("a"), set(&["team-a"]));
}

#[test]
fn another_list_error_is_unavailable_not_forbidden() {
    let h = Harness::new();
    h.connect("a", &[]);
    h.connector
        .ports_for(&id("a"))
        .resources
        .script()
        .list_metadata
        .push_err(OxiError::network("connection reset"));

    let catalog = h.run(h.service.catalog(&id("a"))).unwrap();

    assert_eq!(catalog.source, NamespaceSource::Unavailable);
}

#[test]
fn a_session_that_is_not_connected_has_no_list_but_keeps_its_typed_names() {
    let h = Harness::new();
    h.run(h.service.add_typed(&id("a"), "team-a")).unwrap();

    let catalog = h.run(h.service.catalog(&id("a"))).unwrap();

    assert_eq!(catalog.source, NamespaceSource::Unavailable);
    assert_eq!(catalog.names, ["team-a"]);
}

#[test]
fn typed_names_are_validated_remembered_and_removable() {
    let h = Harness::new();
    for bad in [
        "",
        "  ",
        "Has Space",
        "UPPER",
        "-lead",
        "trail-",
        "a/b",
        &"x".repeat(64),
    ] {
        let err = h.run(h.service.add_typed(&id("a"), bad)).expect_err(bad);
        assert_eq!(err.kind(), oxikube_domain::ErrorKind::Validation, "{bad:?}");
    }
    assert!(
        h.state
            .recorded_calls()
            .iter()
            .all(|c| !matches!(c, StateCall::KvSet(..)))
    );

    h.run(h.service.add_typed(&id("a"), " team-b ")).unwrap();
    h.run(h.service.add_typed(&id("a"), "team-a")).unwrap();
    let again = h.run(h.service.add_typed(&id("a"), "team-a")).unwrap();
    assert!(!again.changed, "no duplicate");
    assert_eq!(again.prefs.typed, ["team-a", "team-b"]);

    let removed = h.run(h.service.remove_typed(&id("a"), "team-a")).unwrap();
    assert_eq!(removed.prefs.typed, ["team-b"]);
}

#[test]
fn namespace_name_rules() {
    for ok in ["a", "dev", "kube-system", "a1", "1a", &"x".repeat(63)] {
        assert!(is_valid_namespace_name(ok), "{ok}");
    }
    for bad in ["", "A", "a_b", "a.b", "-a", "a-", " a", &"x".repeat(64)] {
        assert!(!is_valid_namespace_name(bad), "{bad:?}");
    }
}

fn with_accessible(h: &Harness, names: &[&str]) {
    h.manager.set_prefs_table(
        oxikube_ports::ClusterPrefsTable::new(oxikube_ports::ClusterPrefs::default()).with_cluster(
            id("a"),
            oxikube_ports::ClusterPrefs {
                accessible_namespaces: names.iter().map(|n| (*n).to_owned()).collect(),
                ..Default::default()
            },
        ),
    );
}

#[test]
fn a_forbidden_list_offers_the_accessible_namespaces_setting_with_the_typed_names() {
    let h = Harness::new();
    h.connect("a", &["dev"]);
    with_accessible(&h, &["payments", "billing"]);
    h.run(h.service.add_typed(&id("a"), "team-a")).unwrap();
    h.connector
        .ports_for(&id("a"))
        .resources
        .script()
        .list_metadata
        .push_err(OxiError::forbidden("namespaces is forbidden"));

    let catalog = h.run(h.service.catalog(&id("a"))).unwrap();

    assert_eq!(catalog.source, NamespaceSource::Forbidden);
    assert_eq!(catalog.names, ["billing", "payments", "team-a"]);
}

#[test]
fn the_accessible_namespaces_setting_is_offered_when_the_list_cannot_be_read() {
    let h = Harness::new();
    with_accessible(&h, &["payments"]);

    let catalog = h.run(h.service.catalog(&id("a"))).unwrap();

    assert_eq!(catalog.source, NamespaceSource::Unavailable);
    assert_eq!(catalog.names, ["payments"]);
}

#[test]
fn the_accessible_namespaces_setting_does_not_change_a_listed_cluster() {
    let h = Harness::new();
    h.connect("a", &["dev"]);
    with_accessible(&h, &["payments"]);

    let catalog = h.run(h.service.catalog(&id("a"))).unwrap();

    assert_eq!(catalog.source, NamespaceSource::Cluster);
    assert_eq!(catalog.names, ["dev"]);
}

#[test]
fn a_forbidden_list_with_accessible_namespaces_prunes_nothing() {
    let h = Harness::new();
    h.connect("a", &[]);
    with_accessible(&h, &["payments"]);
    h.run(h.service.select(&id("a"), set(&["elsewhere"])))
        .unwrap();
    for _ in 0..2 {
        h.connector
            .ports_for(&id("a"))
            .resources
            .script()
            .list_metadata
            .push_err(OxiError::forbidden("namespaces is forbidden"));
    }

    let reconciled = h.run(h.service.reconcile(&id("a"))).unwrap();

    assert!(reconciled.dropped.is_empty());
    assert_eq!(h.selection("a"), set(&["elsewhere"]));
}
