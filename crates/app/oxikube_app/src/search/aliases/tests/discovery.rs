//! Acceptance: discovery adds plural, singular, short names and Kind per GVR at lower priority,
//! and follows CRDs coming and going.

use oxikube_domain::ids::Gvk;
use oxikube_testkit::kinds::{cert_manager_kinds, kind};

use super::{exact, gvr, stock};
use crate::search::aliases::{AliasSource, AliasTable, Resolution};

#[test]
fn a_crd_is_reachable_by_plural_singular_short_name_kind_and_qualified_name() {
    let table = stock();
    table.set_discovered(&[kind("example.io", "v1", "MyWidget", "widgets")
        .singular("widget")
        .short("wg")
        .build()]);
    let target = gvr("example.io", "v1", "widgets");
    for word in [
        "widgets",
        "widget",
        "wg",
        "mywidget",
        "MyWidget",
        "widgets.example.io",
    ] {
        let Resolution::Exact(entry) = table.resolve(word) else {
            panic!("`{word}` should be exact");
        };
        assert_eq!(entry.target, target, "{word}");
        assert_eq!(entry.source, AliasSource::Discovery, "{word}");
    }
}

#[test]
fn core_types_get_no_qualified_name() {
    let table = stock();
    assert!(!table.resolve("pods.").is_known());
}

#[test]
fn a_builtin_wins_the_names_discovery_also_has_and_they_agree() {
    let table = stock();
    // `deploy` is discovery's short name and a built-in word: one place, no conflict.
    let Resolution::Exact(entry) = table.resolve("deploy") else {
        panic!("deploy is exact");
    };
    assert_eq!(entry.source, AliasSource::BuiltIn);
    assert!(table.conflicts().iter().all(|c| &*c.name != "deploy"));
}

#[test]
fn a_crd_arriving_and_leaving_changes_only_its_own_names() {
    let table = stock();
    let before = table.len();
    let certs = cert_manager_kinds();
    table.apply_kinds_change(&[], &certs);
    assert_eq!(
        exact(&table, "clusterissuers"),
        gvr("cert-manager.io", "v1", "clusterissuers")
    );
    assert!(table.len() > before);

    table.apply_kinds_change(&[Gvk::new("cert-manager.io", "v1", "Issuer")], &[]);
    assert!(!table.resolve("issuers").is_known());
    assert!(table.resolve("clusterissuers").is_known(), "the rest stays");
    assert!(table.resolve("po").is_known(), "other groups are untouched");

    // The last of a group going leaves nothing of it behind.
    table.apply_kinds_change(
        &[
            Gvk::new("cert-manager.io", "v1", "Certificate"),
            Gvk::new("cert-manager.io", "v1", "ClusterIssuer"),
        ],
        &[],
    );
    assert_eq!(table.len(), before);
}

#[test]
fn a_changed_crd_replaces_its_short_names() {
    let table = AliasTable::new();
    table.set_discovered(&[kind("example.io", "v1", "Widget", "widgets")
        .short("wg")
        .build()]);
    table.apply_kinds_change(
        &[],
        &[kind("example.io", "v1", "Widget", "widgets")
            .short("wdg")
            .build()],
    );
    assert!(!table.resolve("wg").is_known());
    assert!(table.resolve("wdg").is_known());
}

#[test]
fn the_preferred_version_is_the_target_whatever_else_is_served() {
    let table = AliasTable::new();
    table.set_discovered(&[
        kind("example.io", "v1beta1", "Widget", "widgets")
            .not_preferred()
            .build(),
        kind("example.io", "v1", "Widget", "widgets").build(),
    ]);
    assert_eq!(exact(&table, "widgets"), gvr("example.io", "v1", "widgets"));
}

#[test]
fn clearing_discovery_leaves_the_builtins() {
    let table = stock();
    table.apply_kinds_change(&[], &cert_manager_kinds());
    table.clear_discovered();
    assert!(!table.resolve("certificates").is_known());
    assert!(table.resolve("po").is_known());
}
