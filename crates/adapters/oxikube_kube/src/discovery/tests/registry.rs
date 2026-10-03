use kube::core::ApiResource;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, Verb, VerbSet};

use crate::discovery::convert::Discovered;
use crate::discovery::registry::Registry;

fn discovered(group: &str, version: &str, kind: &str, preferred: bool) -> Discovered {
    let plural = format!("{}s", kind.to_lowercase());
    Discovered {
        kind: ResourceKind {
            gvk: Gvk::new(group, version, kind),
            preferred,
            plural: plural.clone(),
            singular: kind.to_lowercase(),
            short_names: vec![],
            categories: vec![],
            verbs: VerbSet::from([Verb::Get, Verb::List, Verb::Watch]),
            namespaced: true,
        },
        api_resource: ApiResource {
            group: group.into(),
            version: version.into(),
            api_version: if group.is_empty() {
                version.into()
            } else {
                format!("{group}/{version}")
            },
            kind: kind.into(),
            plural,
        },
    }
}

fn registry(entries: Vec<Discovered>) -> Registry {
    Registry::from_discovered(entries)
}

#[test]
fn snapshot_is_sorted_and_deduplicated() {
    let reg = registry(vec![
        discovered("b.dev", "v1", "B", true),
        discovered("a.dev", "v1", "A", true),
        discovered("a.dev", "v1", "A", false),
    ]);
    let order: Vec<_> = reg.kinds().map(|k| k.gvk.to_string()).collect();
    assert_eq!(order, ["a.dev/v1/A", "b.dev/v1/B"]);
    assert!(
        reg.get(&Gvk::new("a.dev", "v1", "A"))
            .is_some_and(|k| k.preferred),
        "first listing wins"
    );
}

#[test]
fn clones_share_the_snapshot() {
    let reg = registry(vec![discovered("", "v1", "Pod", true)]);
    assert!(reg.clone().ptr_eq(&reg));
    assert!(!Registry::empty().ptr_eq(&reg));
}

#[test]
fn lookup_by_gvk_and_by_preferred_version() {
    let reg = registry(vec![
        discovered("x.dev", "v1beta1", "X", false),
        discovered("x.dev", "v1", "X", true),
    ]);
    assert_eq!(
        reg.get(&Gvk::new("x.dev", "v1beta1", "X"))
            .map(|k| k.preferred),
        Some(false)
    );
    let preferred = Gvk::new("x.dev", "", "X");
    assert_eq!(
        reg.get(&preferred).map(|k| k.gvk.version.to_string()),
        Some("v1".into())
    );
    assert_eq!(
        reg.api_resource(&preferred).map(|a| a.api_version.clone()),
        Some("x.dev/v1".into())
    );
    assert!(reg.get(&Gvk::new("x.dev", "v2", "X")).is_none());
    assert!(reg.get(&Gvk::new("x.dev", "", "Y")).is_none());
}

#[test]
fn identical_snapshots_have_an_empty_diff() {
    let a = registry(vec![discovered("", "v1", "Pod", true)]);
    let b = registry(vec![discovered("", "v1", "Pod", true)]);
    assert!(a.diff(&b).is_empty());
    assert!(Registry::empty().diff(&Registry::empty()).is_empty());
}

#[test]
fn diff_reports_added_removed_and_changed() {
    let before = registry(vec![
        discovered("", "v1", "Pod", true),
        discovered("old.dev", "v1", "Gone", true),
        discovered("zzz.dev", "v1", "Tail", true),
    ]);
    let mut pod = discovered("", "v1", "Pod", true);
    pod.kind.short_names.push("po".into());
    let after = registry(vec![
        pod,
        discovered("new.dev", "v1", "Fresh", true),
        discovered("zzz.dev", "v1", "Tail", true),
    ]);

    let diff = before.diff(&after);
    let names =
        |kinds: &[ResourceKind]| kinds.iter().map(|k| k.gvk.to_string()).collect::<Vec<_>>();
    assert_eq!(names(&diff.added), ["new.dev/v1/Fresh"]);
    assert_eq!(names(&diff.removed), ["old.dev/v1/Gone"]);
    assert_eq!(diff.changed.len(), 1);
    assert!(diff.changed[0].before.short_names.is_empty());
    assert_eq!(diff.changed[0].after.short_names, ["po"]);
}

#[test]
fn diff_from_empty_adds_everything_and_to_empty_removes_everything() {
    let reg = registry(vec![
        discovered("", "v1", "Pod", true),
        discovered("a.dev", "v1", "A", true),
    ]);
    assert_eq!(Registry::empty().diff(&reg).added.len(), 2);
    assert_eq!(reg.diff(&Registry::empty()).removed.len(), 2);
}

#[test]
fn a_new_preferred_version_is_a_change() {
    let before = registry(vec![
        discovered("x.dev", "v1", "X", true),
        discovered("x.dev", "v2", "X", false),
    ]);
    let after = registry(vec![
        discovered("x.dev", "v1", "X", false),
        discovered("x.dev", "v2", "X", true),
    ]);
    let diff = before.diff(&after);
    assert!(diff.added.is_empty() && diff.removed.is_empty());
    assert_eq!(diff.changed.len(), 2);
}
