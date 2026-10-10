//! Acceptance: collisions are resolved by fixed rules and listed, never silent; unknown names get
//! suggestions.

use oxikube_domain::AliasTarget;
use oxikube_testkit::kinds::{cert_manager_kinds, clashing_crds};

use super::{exact, gvr, stock};
use crate::search::aliases::{AliasSource, AliasTable, ConflictKind, Resolution};

fn with_clashes() -> AliasTable {
    let table = stock();
    let mut kinds = oxikube_testkit::kinds::core_kinds();
    kinds.extend(cert_manager_kinds());
    kinds.extend(clashing_crds());
    table.set_discovered(&kinds);
    table
}

fn groups(resolution: &Resolution) -> Vec<String> {
    let Resolution::Ambiguous(candidates) = resolution else {
        panic!("expected an ambiguous name, got {resolution:?}");
    };
    candidates
        .iter()
        .map(|e| match &e.target {
            AliasTarget::Gvr(g) => g.group.to_string(),
            AliasTarget::Command { .. } => unreachable!(),
        })
        .collect()
}

#[test]
fn two_crds_sharing_a_plural_are_ambiguous_in_a_fixed_order() {
    let table = with_clashes();
    let resolution = table.resolve("certificates");
    assert_eq!(groups(&resolution), ["cert-manager.io", "example.io"]);
    // The first candidate is what a caller that wants one place gets, every time.
    assert_eq!(
        resolution.target(),
        Some(&gvr("cert-manager.io", "v1", "certificates"))
    );

    let conflict = table
        .conflicts()
        .into_iter()
        .find(|c| &*c.name == "certificates")
        .expect("listed");
    assert_eq!(conflict.kind, ConflictKind::Ambiguous);
    assert_eq!(conflict.others.len(), 1);
}

#[test]
fn a_qualified_name_picks_one_of_the_colliding_groups() {
    let table = with_clashes();
    assert_eq!(
        exact(&table, "certificates.example.io"),
        gvr("example.io", "v1", "certificates")
    );
    assert_eq!(
        exact(&table, "certificates.cert-manager.io"),
        gvr("cert-manager.io", "v1", "certificates")
    );
}

#[test]
fn a_shared_short_name_is_ambiguous_too() {
    let table = with_clashes();
    assert_eq!(
        groups(&table.resolve("cert")),
        ["cert-manager.io", "example.io"]
    );
}

#[test]
fn a_crd_short_name_equal_to_a_builtin_loses_and_is_reported() {
    let table = with_clashes();
    let Resolution::Exact(entry) = table.resolve("dp") else {
        panic!("dp is exact");
    };
    assert_eq!(entry.source, AliasSource::BuiltIn);
    assert_eq!(entry.target, gvr("apps", "v1", "deployments"));

    let conflict = table
        .conflicts()
        .into_iter()
        .find(|c| &*c.name == "dp")
        .expect("listed");
    assert_eq!(conflict.kind, ConflictKind::Shadowed);
    assert_eq!(conflict.others.len(), 1);
    assert_eq!(
        conflict.others[0].target,
        gvr("rollouts.io", "v1alpha1", "rollouts")
    );
    // The CRD is still reachable by its other names.
    assert!(table.resolve("rollouts").is_known());
}

#[test]
fn the_core_and_events_api_agree_on_the_builtin_and_split_on_the_rest() {
    let table = stock();
    // `ev` and `events` are built in: core wins, `events.k8s.io` is the loser.
    assert_eq!(exact(&table, "ev"), gvr("", "v1", "events"));
    let conflict = table
        .conflicts()
        .into_iter()
        .find(|c| &*c.name == "events")
        .expect("events is contested");
    assert_eq!(conflict.kind, ConflictKind::Shadowed);
    assert_eq!(
        exact(&table, "events.events.k8s.io"),
        gvr("events.k8s.io", "v1", "events")
    );
}

#[test]
fn conflicts_come_out_sorted_by_name() {
    let table = with_clashes();
    let names: Vec<_> = table.conflicts().into_iter().map(|c| c.name).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[test]
fn an_unknown_name_gets_suggestions_by_prefix_and_by_edit_distance() {
    let table = stock();
    let Resolution::Unknown { suggestions } = table.resolve("deploymnt") else {
        panic!("deploymnt is unknown");
    };
    assert!(
        suggestions.iter().any(|s| &**s == "deployment"),
        "{suggestions:?}"
    );

    let Resolution::Unknown { suggestions } = table.resolve("stat") else {
        panic!("stat is unknown");
    };
    assert!(
        suggestions.iter().any(|s| &**s == "statefulset"),
        "{suggestions:?}"
    );
    assert!(suggestions.len() <= crate::search::aliases::MAX_SUGGESTIONS);

    let Resolution::Unknown { suggestions } = table.resolve("qqqqqqqq") else {
        panic!("unknown");
    };
    assert!(suggestions.is_empty());
    assert!(matches!(table.resolve(""), Resolution::Unknown { .. }));
}

#[test]
fn suggestions_lead_with_the_shortest_completion() {
    let table = stock();
    let Resolution::Unknown { suggestions } = table.resolve("dep") else {
        panic!("dep is unknown");
    };
    assert_eq!(&*suggestions[0], "deploy");
}
