//! Acceptance: the built-in k9s aliases, and the user layer above them.

use oxikube_domain::AliasTarget;

use super::{exact, gvr, stock};
use crate::search::aliases::{AliasSource, AliasTable, ConflictKind, Resolution};

/// Every alias the story lists, and the type it must open (at the default version, before the
/// cluster has answered discovery).
const ACCEPTANCE: &[(&str, &str, &str, &str)] = &[
    ("po", "", "v1", "pods"),
    ("dp", "apps", "v1", "deployments"),
    ("svc", "", "v1", "services"),
    ("sts", "apps", "v1", "statefulsets"),
    ("ds", "apps", "v1", "daemonsets"),
    ("cj", "batch", "v1", "cronjobs"),
    ("ing", "networking.k8s.io", "v1", "ingresses"),
    ("np", "networking.k8s.io", "v1", "networkpolicies"),
    ("pv", "", "v1", "persistentvolumes"),
    ("pvc", "", "v1", "persistentvolumeclaims"),
    ("sc", "storage.k8s.io", "v1", "storageclasses"),
    ("cm", "", "v1", "configmaps"),
    ("sec", "", "v1", "secrets"),
    ("sa", "", "v1", "serviceaccounts"),
    ("ro", "rbac.authorization.k8s.io", "v1", "roles"),
    ("rb", "rbac.authorization.k8s.io", "v1", "rolebindings"),
    ("cr", "rbac.authorization.k8s.io", "v1", "clusterroles"),
    (
        "crb",
        "rbac.authorization.k8s.io",
        "v1",
        "clusterrolebindings",
    ),
    ("hpa", "autoscaling", "v2", "horizontalpodautoscalers"),
    ("pdb", "policy", "v1", "poddisruptionbudgets"),
    ("ev", "", "v1", "events"),
    ("no", "", "v1", "nodes"),
    ("ns", "", "v1", "namespaces"),
    (
        "crd",
        "apiextensions.k8s.io",
        "v1",
        "customresourcedefinitions",
    ),
    // The rest of k9s's section C.
    ("rs", "apps", "v1", "replicasets"),
    ("rc", "", "v1", "replicationcontrollers"),
    ("job", "batch", "v1", "jobs"),
];

#[test]
fn the_builtin_k9s_aliases_resolve_without_any_discovery() {
    let table = AliasTable::new();
    for (name, group, version, resource) in ACCEPTANCE {
        let Resolution::Exact(entry) = table.resolve(name) else {
            panic!("`{name}` should resolve");
        };
        assert_eq!(entry.source, AliasSource::BuiltIn, "{name}");
        assert_eq!(entry.target, gvr(group, version, resource), "{name}");
        assert_eq!(&*entry.name, *name);
    }
}

#[test]
fn plural_and_singular_of_core_types_work_before_the_cluster_answers() {
    let table = AliasTable::new();
    for word in ["pods", "pod", "deployments", "deploy", "configmap", "nodes"] {
        assert!(table.resolve(word).is_known(), "{word}");
    }
}

#[test]
fn lookup_ignores_case_and_surrounding_blanks() {
    let table = stock();
    let expected = gvr("apps", "v1", "deployments");
    for word in ["DP", "Dp", " dp ", "DEPLOY", "Deployments"] {
        assert_eq!(exact(&table, word), expected, "{word:?}");
    }
}

#[test]
fn a_builtin_follows_the_version_the_cluster_serves() {
    let table = AliasTable::new();
    assert_eq!(
        exact(&table, "hpa"),
        gvr("autoscaling", "v2", "horizontalpodautoscalers")
    );
    // An old cluster that only serves v2beta2.
    table.set_discovered(&[oxikube_testkit::kinds::kind(
        "autoscaling",
        "v2beta2",
        "HorizontalPodAutoscaler",
        "horizontalpodautoscalers",
    )
    .short("hpa")
    .build()]);
    assert_eq!(
        exact(&table, "hpa"),
        gvr("autoscaling", "v2beta2", "horizontalpodautoscalers")
    );
    table.clear_discovered();
    assert_eq!(
        exact(&table, "hpa"),
        gvr("autoscaling", "v2", "horizontalpodautoscalers")
    );
}

#[test]
fn a_user_alias_beats_a_builtin_and_the_conflict_is_listed() {
    let table = stock();
    table.set_user_aliases([("PO".to_owned(), gvr("", "v1", "persistentvolumes"))]);

    let Resolution::Exact(entry) = table.resolve("po") else {
        panic!("po is exact");
    };
    assert_eq!(entry.source, AliasSource::User);
    assert_eq!(entry.target, gvr("", "v1", "persistentvolumes"));

    let conflicts = table.conflicts();
    let po = conflicts
        .iter()
        .find(|c| &*c.name == "po")
        .expect("po conflicts");
    assert_eq!(po.kind, ConflictKind::Shadowed);
    assert_eq!(po.winner.source, AliasSource::User);
    assert!(po.others.iter().any(|e| e.source == AliasSource::BuiltIn));
    // `pod` is untouched.
    assert_eq!(exact(&table, "pod"), gvr("", "v1", "pods"));
}

#[test]
fn a_user_alias_may_name_a_command_with_arguments() {
    let table = AliasTable::new();
    let fred = AliasTarget::command("pod", ["fred".to_owned(), "app=blee".to_owned()]);
    table.set_user_aliases([("fred".to_owned(), fred.clone())]);
    assert_eq!(exact(&table, "fred"), fred);
}

#[test]
fn replacing_the_user_layer_drops_the_old_entries_and_skips_bad_names() {
    let table = AliasTable::new();
    table.set_user_aliases([
        ("mine".to_owned(), gvr("", "v1", "pods")),
        ("   ".to_owned(), gvr("", "v1", "pods")),
        ("has space".to_owned(), gvr("", "v1", "pods")),
        ("dup".to_owned(), gvr("", "v1", "pods")),
        ("dup".to_owned(), gvr("", "v1", "nodes")),
    ]);
    assert!(table.resolve("mine").is_known());
    assert!(!table.resolve("has space").is_known());
    assert_eq!(
        exact(&table, "dup"),
        gvr("", "v1", "nodes"),
        "the last wins"
    );

    table.set_user_aliases([]);
    assert!(!table.resolve("mine").is_known());
    assert!(table.resolve("po").is_known(), "built-ins stay");
}

#[test]
fn the_builtin_names_are_unique_and_lower_case() {
    let mut seen = std::collections::BTreeSet::new();
    for row in super::super::builtin::BUILTINS {
        for name in row.names {
            assert_eq!(*name, name.to_ascii_lowercase());
            assert!(seen.insert(*name), "`{name}` is in two rows");
        }
    }
}
