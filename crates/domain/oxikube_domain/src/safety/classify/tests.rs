use super::*;
use crate::command::{Propagation, delete_risk};

fn gvk(group: &str, kind: &str) -> Gvk {
    Gvk::new(group, "v1", kind)
}

fn risk(verb: MutationVerb, scope: Scope, group: &str, kind: &str) -> Risk {
    classify(verb, scope, &gvk(group, kind), MutationFlags::default())
}

#[test]
fn single_namespaced_writes_of_built_in_kinds_are_low() {
    for verb in [
        MutationVerb::Create,
        MutationVerb::Update,
        MutationVerb::Patch,
        MutationVerb::Apply,
        MutationVerb::Scale,
        MutationVerb::Restart,
    ] {
        for (group, kind) in [("", "Pod"), ("", "ConfigMap"), ("apps", "Deployment")] {
            assert_eq!(
                risk(verb, Scope::Namespaced, group, kind),
                Risk::Low,
                "{verb} {kind}"
            );
        }
    }
}

#[test]
fn delete_and_evict_are_medium_for_ordinary_kinds() {
    for verb in [MutationVerb::Delete, MutationVerb::Evict] {
        assert_eq!(risk(verb, Scope::Namespaced, "", "Pod"), Risk::Medium);
        assert_eq!(
            risk(verb, Scope::Namespaced, "apps", "Deployment"),
            Risk::Medium
        );
    }
}

#[test]
fn bulk_and_node_verbs() {
    assert_eq!(
        risk(MutationVerb::DeleteCollection, Scope::Namespaced, "", "Pod"),
        Risk::High
    );
    assert_eq!(
        risk(MutationVerb::Prune, Scope::Namespaced, "apps", "Deployment"),
        Risk::High
    );
    assert_eq!(
        risk(MutationVerb::Drain, Scope::Cluster, "", "Node"),
        Risk::High
    );
    assert_eq!(
        risk(MutationVerb::Cordon, Scope::Cluster, "", "Node"),
        Risk::Medium
    );
    assert_eq!(
        risk(MutationVerb::Uncordon, Scope::Cluster, "", "Node"),
        Risk::Medium
    );
}

#[test]
fn the_type_the_name_kinds() {
    for verb in [MutationVerb::Delete, MutationVerb::DeleteCollection] {
        assert_eq!(
            risk(verb, Scope::Cluster, "", "Namespace"),
            Risk::Irreversible
        );
        assert_eq!(
            risk(verb, Scope::Cluster, "", "PersistentVolume"),
            Risk::Irreversible
        );
    }
    assert_eq!(
        risk(MutationVerb::Delete, Scope::Cluster, "", "Node"),
        Risk::High
    );
    assert_eq!(
        risk(
            MutationVerb::Delete,
            Scope::Cluster,
            "apiextensions.k8s.io",
            "CustomResourceDefinition"
        ),
        Risk::High
    );
    for (verb, kind) in [
        (MutationVerb::Delete, "Namespace"),
        (MutationVerb::Delete, "Node"),
    ] {
        assert_eq!(
            risk(verb, Scope::Cluster, "", kind).confirm_tier(),
            ConfirmTier::TypeName
        );
    }
}

use super::super::ConfirmTier;

#[test]
fn rbac_writes_are_medium_and_reads_of_the_rule_do_not_leak() {
    for kind in ["Role", "ClusterRole", "RoleBinding", "ClusterRoleBinding"] {
        for verb in [
            MutationVerb::Create,
            MutationVerb::Patch,
            MutationVerb::Apply,
        ] {
            let scope = if kind.starts_with("Cluster") {
                Scope::Cluster
            } else {
                Scope::Namespaced
            };
            assert_eq!(
                risk(verb, scope, RBAC_GROUP, kind),
                Risk::Medium,
                "{verb} {kind}"
            );
        }
    }
    // The same kind name in another group is an ordinary custom resource (still medium, not low).
    assert_eq!(
        risk(
            MutationVerb::Create,
            Scope::Namespaced,
            "example.io",
            "Role"
        ),
        Risk::Medium
    );
}

#[test]
fn unknown_kinds_default_to_medium_never_low() {
    for verb in MutationVerb::ALL {
        assert!(
            risk(verb, Scope::Namespaced, "example.io", "Widget") >= Risk::Medium,
            "{verb}"
        );
    }
}

#[test]
fn cluster_scoped_writes_are_at_least_medium() {
    assert_eq!(
        risk(
            MutationVerb::Create,
            Scope::Cluster,
            "storage.k8s.io",
            "StorageClass"
        ),
        Risk::Medium
    );
}

#[test]
fn flags_raise_the_floor() {
    let pod = gvk("", "Pod");
    let with = |verb, flags| classify(verb, Scope::Namespaced, &pod, flags);
    let flag = |f: fn(&mut MutationFlags)| {
        let mut flags = MutationFlags::default();
        f(&mut flags);
        flags
    };
    assert_eq!(
        with(MutationVerb::Delete, flag(|f| f.cascading = true)),
        Risk::High
    );
    assert_eq!(
        with(MutationVerb::Delete, flag(|f| f.force = true)),
        Risk::High
    );
    assert_eq!(
        with(MutationVerb::Evict, flag(|f| f.force = true)),
        Risk::High
    );
    assert_eq!(
        with(MutationVerb::Scale, flag(|f| f.to_zero = true)),
        Risk::Medium
    );
    assert_eq!(
        with(MutationVerb::Scale, flag(|f| f.force = true)),
        Risk::Low
    );
    assert_eq!(
        with(MutationVerb::Delete, flag(|f| f.all_namespaces = true)),
        Risk::High
    );
    assert_eq!(
        with(MutationVerb::Patch, flag(|f| f.all_namespaces = true)),
        Risk::Medium
    );
    assert_eq!(
        with(MutationVerb::Apply, flag(|f| f.force = true)),
        Risk::Medium
    );
}

#[test]
fn flags_never_lower_a_risk() {
    for verb in MutationVerb::ALL {
        for (group, kind) in [("", "Pod"), ("", "Namespace"), ("example.io", "Widget")] {
            let base = risk(verb, Scope::Namespaced, group, kind);
            for bits in 0..16u8 {
                let flags = MutationFlags {
                    cascading: bits & 1 != 0,
                    force: bits & 2 != 0,
                    all_namespaces: bits & 4 != 0,
                    to_zero: bits & 8 != 0,
                };
                assert!(classify(verb, Scope::Namespaced, &gvk(group, kind), flags) >= base);
            }
        }
    }
}

#[test]
fn classify_agrees_with_delete_risk() {
    for (group, kind) in [
        ("", "Pod"),
        ("", "Namespace"),
        ("", "PersistentVolume"),
        ("", "Node"),
        ("apps", "Deployment"),
        ("example.io", "Node"),
        ("example.io", "Widget"),
        ("rbac.authorization.k8s.io", "ClusterRole"),
        ("apiextensions.k8s.io", "CustomResourceDefinition"),
    ] {
        for propagation in [
            Propagation::Background,
            Propagation::Foreground,
            Propagation::Orphan,
        ] {
            for scope in [Scope::Namespaced, Scope::Cluster] {
                let flags = MutationFlags {
                    cascading: propagation == Propagation::Foreground,
                    ..MutationFlags::default()
                };
                assert_eq!(
                    classify(MutationVerb::Delete, scope, &gvk(group, kind), flags),
                    delete_risk(&gvk(group, kind), propagation),
                    "{group}/{kind} {propagation:?} {scope:?}"
                );
            }
        }
    }
}

#[test]
fn verb_floor_is_the_lowest_result_for_a_verb() {
    for verb in MutationVerb::ALL {
        assert!(risk(verb, Scope::Namespaced, "", "Pod") >= verb_floor(verb));
    }
}
