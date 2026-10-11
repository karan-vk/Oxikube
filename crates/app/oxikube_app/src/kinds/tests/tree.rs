//! The sidebar tree: section and entry order (stable whatever the registration order), search,
//! cost.

use std::time::Instant;

use oxikube_domain::ids::{Gvk, Scope};
use proptest::prelude::*;

use super::built_ins;
use crate::kinds::{KindMeta, KindRegistry, Section, SidebarTree};

fn tree_of(metas: Vec<KindMeta>) -> SidebarTree {
    let mut registry = KindRegistry::new();
    for meta in metas {
        registry.register(meta).expect("registers");
    }
    registry.sidebar_tree(None)
}

/// `(section, plural)` in sidebar order.
fn order(tree: &SidebarTree) -> Vec<(&'static str, String)> {
    tree.entries()
        .map(|(section, entry)| (section.id, entry.meta.plural.to_string()))
        .collect()
}

#[test]
fn sections_and_entries_follow_their_ranks() {
    let tree = tree_of(built_ins());
    let sections: Vec<_> = tree.sections.iter().map(|s| s.section.id).collect();
    assert_eq!(
        sections,
        ["nodes", "workloads", "config", "network", "namespaces"]
    );
    let workloads: Vec<_> = tree
        .section("workloads")
        .expect("workloads")
        .entries
        .iter()
        .map(|e| e.meta.plural.to_string())
        .collect();
    // By rank, not by registration order (StatefulSets was registered before DaemonSets).
    assert_eq!(
        workloads,
        ["pods", "deployments", "daemonsets", "statefulsets"]
    );
}

#[test]
fn a_kind_without_a_slot_is_not_in_the_sidebar() {
    let mut metas = built_ins();
    metas.push(KindMeta::new(
        Gvk::new("apiextensions.k8s.io", "v1", "CustomResourceDefinition"),
        "customresourcedefinitions",
        "Definitions",
        Scope::Cluster,
    ));
    let tree = tree_of(metas);
    assert_eq!(tree.len(), built_ins().len());
}

#[test]
fn equal_ranks_are_ordered_by_title() {
    let at = |kind: &str, plural: &str| {
        KindMeta::new(
            Gvk::new("x.io", "v1", kind),
            plural,
            kind,
            Scope::Namespaced,
        )
        .in_section(Section::STORAGE, 5)
    };
    let tree = tree_of(vec![at("Zebra", "zebras"), at("Alpha", "alphas")]);
    assert_eq!(
        order(&tree),
        [
            ("storage", "alphas".to_owned()),
            ("storage", "zebras".to_owned())
        ]
    );
}

#[test]
fn search_matches_title_kind_and_plural() {
    let tree = tree_of(built_ins());
    let plurals = |query: &str| -> Vec<String> {
        tree.search(query)
            .into_iter()
            .map(|(_, e)| e.meta.plural.to_string())
            .collect()
    };
    assert_eq!(plurals("deploy"), ["deployments"]);
    assert_eq!(plurals("SET"), ["daemonsets", "statefulsets"]);
    assert_eq!(plurals("gateways"), ["gateways"]);
    assert_eq!(plurals("").len(), built_ins().len());
    assert!(plurals("nothing-like-it").is_empty());
}

/// About as many kinds as the built-in catalogue registers (and then some): the tree is built
/// once per discovery change, so it must stay far below a frame.
#[test]
fn building_the_tree_of_eighty_kinds_is_cheap() {
    let sections = Section::BUILT_IN;
    let mut registry = KindRegistry::new();
    let started = Instant::now();
    for i in 0..80u32 {
        let kind = format!("Kind{i}");
        let meta = KindMeta::new(
            Gvk::new("bench.io", "v1", kind.as_str()),
            &format!("kind{i}s"),
            &kind,
            Scope::Namespaced,
        )
        .in_section(sections[(i as usize) % sections.len()], i);
        registry.register(meta).expect("registers");
    }
    let registered = started.elapsed();
    let started = Instant::now();
    let tree = registry.sidebar_tree(None);
    let built = started.elapsed();
    assert_eq!(tree.len(), 80);
    // Generous bounds for a debug build on a busy CI runner; the measured cost is microseconds.
    assert!(
        registered.as_millis() < 50,
        "registering took {registered:?}"
    );
    assert!(built.as_millis() < 20, "building the tree took {built:?}");
}

proptest! {
    /// Order of registration never changes the sidebar.
    #[test]
    fn the_order_is_stable_under_shuffled_registration(seed in any::<u64>()) {
        let mut metas = built_ins();
        // A deterministic shuffle from the seed (Fisher-Yates over a xorshift).
        let mut state = seed | 1;
        for i in (1..metas.len()).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            metas.swap(i, (state % (i as u64 + 1)) as usize);
        }
        prop_assert_eq!(order(&tree_of(metas)), order(&tree_of(built_ins())));
    }
}
