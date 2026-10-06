//! The versions of a kind that the cluster serves, for the table's switcher.

use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, VerbSet};

use crate::crds::served_versions;

fn kind(group: &str, version: &str, name: &str, preferred: bool, verbs: &[&str]) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, version, name),
        preferred,
        plural: format!("{}s", name.to_lowercase()),
        singular: name.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(verbs.iter().copied()),
        namespaced: true,
    }
}

#[test]
fn the_versions_of_one_kind_come_newest_first_and_other_kinds_are_left_out() {
    let list = ["get", "list", "watch"];
    let discovered = [
        kind("example.com", "v1beta1", "Widget", false, &list),
        kind("example.com", "v1", "Widget", true, &list),
        kind("example.com", "v1", "Gadget", true, &list),
        kind("other.io", "v1", "Widget", true, &list),
        kind("example.com", "v1alpha1", "Widget", false, &["get"]),
    ];
    let versions = served_versions(&discovered, &discovered[1]);
    let found: Vec<_> = versions.iter().map(|k| k.gvk.version.to_string()).collect();
    assert_eq!(
        found,
        ["v1", "v1beta1"],
        "not Gadget, not the other group, not the unlistable"
    );
    assert!(versions[0].preferred);
}

#[test]
fn a_kind_with_one_version_has_one_entry() {
    let list = ["list"];
    let only = kind("example.com", "v1", "Widget", true, &list);
    assert_eq!(served_versions(std::slice::from_ref(&only), &only).len(), 1);
    assert_eq!(served_versions(&[], &only).len(), 0);
}
