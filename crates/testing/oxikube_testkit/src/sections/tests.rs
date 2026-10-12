use std::collections::BTreeSet;

use oxikube_domain::ids::Scope;

use super::*;

/// The sidebar sections in the order the sidebar lists them.
const SECTIONS: &[&str] = &[
    "nodes",
    "workloads",
    "config",
    "network",
    "storage",
    "namespaces",
    "custom-resources",
];

const SEED: Seed<'static> = Seed {
    namespace: "oxi-test-0123abcd",
    stem: "oxi-test-0123abcd",
};

#[test]
fn every_seed_is_an_object_of_its_kind_with_its_name() {
    for case in catalog() {
        let object = seed_resource(case, &SEED);
        assert_eq!(object.kind, case.gvk(), "{}", case.kind);
        assert_eq!(object.name(), case.seed_name(&SEED), "{}", case.kind);
        let namespace = case.namespaced().then_some(SEED.namespace);
        assert_eq!(object.namespace(), namespace, "{}", case.kind);
        assert_eq!(
            case.seed(&SEED)["metadata"]["labels"][SUITE_LABEL],
            "sections",
            "{}",
            case.kind
        );
    }
}

#[test]
fn names_are_unique_dns_names_and_cluster_scoped_ones_carry_the_stem() {
    let mut names = BTreeSet::new();
    for case in catalog() {
        let name = case.seed_name(&SEED);
        assert!(name.len() <= 63, "{name}");
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.'),
            "{name}"
        );
        if case.scope == Scope::Cluster {
            assert!(name.contains(SEED.stem), "{name}");
        }
        assert!(names.insert((case.kind, name.clone())), "{name} twice");
    }
}

#[test]
fn the_catalog_is_in_sidebar_order_with_one_case_per_entry() {
    let mut entries = BTreeSet::new();
    let mut section = 0;
    for case in catalog() {
        let at = SECTIONS
            .iter()
            .position(|s| *s == case.section)
            .unwrap_or_else(|| panic!("{} is in no known section", case.kind));
        assert!(at >= section, "{} is out of order", case.kind);
        section = at;
        assert!(entries.insert(case.entry()), "{} twice", case.entry());
        assert_eq!(case.columns.first(), Some(&"Name"), "{}", case.kind);
        assert!(super::case(case.kind).is_some());
    }
    // The 40 kinds of the built-in sections and the CRD list.
    assert_eq!(entries.len(), 41);
}

#[test]
fn no_seed_runs_a_pod_outside_the_pre_pulled_images_or_schedules_a_controller() {
    for case in catalog() {
        let text = case.seed(&SEED).to_string();
        if text.contains("\"image\"") {
            assert!(text.contains(crate::images::PAUSE), "{}", case.kind);
        }
        let spec = &case.seed(&SEED)["spec"];
        if let Some(replicas) = spec.get("replicas") {
            assert_eq!(replicas, 0, "{} has no replicas", case.kind);
        }
    }
    let job = case("Job").expect("Jobs").seed(&SEED);
    assert_eq!(job["spec"]["suspend"], true);
    let cronjob = case("CronJob").expect("CronJobs").seed(&SEED);
    assert_eq!(cronjob["spec"]["suspend"], true);
}

#[test]
fn optional_kinds_are_the_add_ons_apis() {
    let optional: Vec<&str> = catalog()
        .filter(|c| c.api == Api::Optional)
        .map(|c| c.group)
        .collect();
    assert!(optional.iter().all(|g| {
        [
            "gateway.networking.k8s.io",
            "autoscaling.k8s.io",
            "snapshot.storage.k8s.io",
        ]
        .contains(g)
    }));
    assert_eq!(optional.len(), 8);
}
