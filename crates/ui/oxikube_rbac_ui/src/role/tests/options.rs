//! The resource picker's choices and the verbs a pick supports.

use oxikube_domain::kinds::{ResourceKind, Verb};
use oxikube_testkit::kinds::{KindSpec, core_kinds};

use crate::role::ResourceOptions;

fn options() -> ResourceOptions {
    let mut kinds = core_kinds();
    // A CRD that cannot be deleted, and a second, non-preferred version that is ignored.
    kinds.push(
        KindSpec::new("example.com", "v1", "Widget", "widgets")
            .list_only()
            .build(),
    );
    kinds.push(
        KindSpec::new("apps", "v1beta1", "Deployment", "deployments")
            .not_preferred()
            .build(),
    );
    ResourceOptions::from_kinds(&kinds)
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn every_served_resource_is_offered_once_with_its_group_and_the_subresources() {
    let options = options();
    let labels: Vec<String> = options.suggest("", 500).iter().map(|o| o.label()).collect();
    assert!(labels.contains(&"pods".to_owned()));
    assert!(labels.contains(&"apps/deployments".to_owned()));
    assert!(
        labels.contains(&"pods/log".to_owned()),
        "subresources are added"
    );
    assert!(labels.contains(&"apps/deployments/scale".to_owned()));
    assert!(
        labels.contains(&"example.com/widgets".to_owned()),
        "CRDs are listed"
    );
    assert_eq!(
        labels
            .iter()
            .filter(|l| l.as_str() == "apps/deployments")
            .count(),
        1,
        "the non-preferred version is dropped"
    );
    let mut sorted = labels.clone();
    sorted.sort();
    assert_eq!(labels, sorted);
}

#[test]
fn suggestions_start_with_the_prefix_matches_and_are_capped() {
    let options = options();
    let found: Vec<String> = options
        .suggest("dep", 3)
        .iter()
        .map(|o| o.label())
        .collect();
    assert_eq!(found[0], "apps/deployments");
    assert!(found.len() <= 3);
    let by_group: Vec<String> = options
        .suggest("example", 5)
        .iter()
        .map(|o| o.label())
        .collect();
    assert_eq!(by_group, ["example.com/widgets"]);
    assert!(options.suggest("nothing-like-this", 5).is_empty());
}

#[test]
fn only_the_verbs_the_picked_kinds_support_are_offered() {
    let options = options();
    let widgets = options
        .supported_verbs(&strings(&["example.com"]), &strings(&["widgets"]))
        .expect("known");
    assert!(widgets.contains(Verb::Get) && widgets.contains(Verb::List));
    assert!(!widgets.contains(Verb::Delete), "list-only kind");
    // A subresource takes its own verbs.
    let log = options
        .supported_verbs(&strings(&[""]), &strings(&["pods/log"]))
        .expect("known");
    assert!(log.contains(Verb::Get) && !log.contains(Verb::List));
    // Several resources: the union.
    let both = options
        .supported_verbs(
            &strings(&["", "example.com"]),
            &strings(&["pods", "widgets"]),
        )
        .expect("known");
    assert!(both.contains(Verb::Delete), "pods can be deleted");
}

#[test]
fn wildcards_unknown_names_and_no_resources_restrict_nothing() {
    let options = options();
    for resources in [&["*"][..], &["*/scale"][..], &["gizmos"][..], &[][..]] {
        assert_eq!(
            options.supported_verbs(&strings(&[""]), &strings(resources)),
            None,
            "{resources:?}"
        );
    }
    // One unknown name among known ones restricts nothing either.
    assert_eq!(
        options.supported_verbs(&strings(&[""]), &strings(&["pods", "gizmos"])),
        None
    );
    // A resource in another group than the one typed is not the kind picked.
    assert_eq!(
        options.supported_verbs(&strings(&["example.com"]), &strings(&["pods"])),
        None
    );
}

#[test]
fn the_group_of_a_resource_and_its_scope_come_from_discovery() {
    let options = options();
    assert_eq!(options.group_of("deployments"), Some("apps"));
    assert_eq!(options.group_of("pods"), Some(""));
    assert_eq!(options.group_of("nothing"), None);
    assert_eq!(options.is_namespaced("", "pods"), Some(true));
    assert_eq!(options.is_namespaced("", "nodes"), Some(false));
    assert_eq!(options.is_namespaced("", "gizmos"), None);
    assert!(ResourceOptions::from_kinds(&[] as &[ResourceKind]).is_empty());
}
