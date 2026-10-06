//! The visibility rules, pure: `build_rows` over sections, access, discovery and open state.

use std::collections::BTreeMap;

use oxikube_app::{AccessOutcome, CustomKind, CustomResourceGroup, IntegrationSection};
use oxikube_domain::access::AccessRules;
use oxikube_domain::command::CommandId;
use oxikube_ports::{SidebarItem, SidebarSection as PortSection};

use super::can_list;
use crate::sidebar::{
    AccessState, NoticeKind, Row, RowInputs, SidebarTarget, build_rows, core_sections,
};

fn known(rules: AccessRules) -> AccessState {
    AccessState::Known(AccessOutcome::Reviewed(rules))
}

fn rows_for(
    access: &AccessState,
    custom: Option<&[CustomResourceGroup]>,
    open: &BTreeMap<String, bool>,
    integrations: &[IntegrationSection],
) -> Vec<Row> {
    build_rows(&RowInputs {
        sections: &core_sections(),
        integrations,
        custom,
        access,
        open,
    })
}

fn section_ids(rows: &[Row]) -> Vec<&str> {
    rows.iter()
        .filter_map(|r| match r {
            Row::Section(s) => Some(&*s.id),
            _ => None,
        })
        .collect()
}

fn argo() -> Vec<CustomResourceGroup> {
    vec![CustomResourceGroup {
        group: "argoproj.io".into(),
        kinds: vec![
            CustomKind {
                kind: "AppProject".into(),
                plural: "appprojects".into(),
                version: "v1alpha1".into(),
                namespaced: true,
            },
            CustomKind {
                kind: "Application".into(),
                plural: "applications".into(),
                version: "v1alpha1".into(),
                namespaced: true,
            },
        ],
    }]
}

#[test]
fn the_core_sections_are_the_eleven_lens_groups_in_order() {
    let ids: Vec<_> = core_sections()
        .into_iter()
        .map(|s| s.id.to_string())
        .collect();
    assert_eq!(
        ids,
        [
            "cluster",
            "nodes",
            "workloads",
            "config",
            "network",
            "storage",
            "namespaces",
            "events",
            "helm",
            "access-control",
            "custom-resources",
        ]
    );
}

#[test]
fn a_full_admin_sees_every_section_with_a_count_placeholder() {
    let rows = rows_for(
        &known(AccessRules::all_access()),
        Some(&argo()),
        &BTreeMap::new(),
        &[],
    );
    assert_eq!(section_ids(&rows).len(), 11);
    for row in &rows {
        if let Row::Section(section) = row {
            assert_eq!(section.count, None, "{}: counts come later", section.id);
        }
    }
    assert!(
        !rows.iter().any(|r| matches!(r, Row::Notice(_))),
        "nothing hidden, nothing to say"
    );
}

#[test]
fn a_restricted_user_sees_only_the_sections_they_may_list() {
    let access = known(can_list(&[("", "pods"), ("", "services")]));
    let rows = rows_for(&access, Some(&argo()), &BTreeMap::new(), &[]);
    assert_eq!(section_ids(&rows), ["cluster", "workloads", "network"]);
    // Inside a visible section only the listable kinds show.
    let entries: Vec<_> = rows
        .iter()
        .filter_map(|r| match r {
            Row::Entry(e) => Some(&*e.id),
            _ => None,
        })
        .collect();
    assert_eq!(entries, ["workloads/pods", "network/services"]);
    // A muted hint says why the rest is missing, instead of leaving an empty page.
    let Some(Row::Notice(notice)) = rows.last() else {
        panic!("a limited-access hint closes the list: {rows:?}");
    };
    assert_eq!(notice.kind, NoticeKind::Muted);
    assert_eq!(notice.id, "access-limited");
    assert!(notice.text.contains("8 sections"), "{}", notice.text);
}

#[test]
fn a_section_needing_any_one_of_two_kinds_shows_for_either() {
    // Events live in two API groups; either is enough. Helm: secrets or configmaps.
    let core_events = rows_for(
        &known(can_list(&[("", "events")])),
        None,
        &BTreeMap::new(),
        &[],
    );
    assert!(section_ids(&core_events).contains(&"events"));
    let new_events = rows_for(
        &known(can_list(&[("events.k8s.io", "events")])),
        None,
        &BTreeMap::new(),
        &[],
    );
    assert!(section_ids(&new_events).contains(&"events"));
    let helm = rows_for(
        &known(can_list(&[("", "configmaps")])),
        None,
        &BTreeMap::new(),
        &[],
    );
    assert!(section_ids(&helm).contains(&"helm"));
    assert!(section_ids(&helm).contains(&"config"));
}

#[test]
fn custom_resources_list_listable_groups_and_hide_when_none_is_listable() {
    let mut open = BTreeMap::new();
    open.insert("crd:argoproj.io".to_owned(), true);
    // Only `applications` is listable: the group shows with that one kind.
    let some = rows_for(
        &known(can_list(&[("argoproj.io", "applications")])),
        Some(&argo()),
        &open,
        &[],
    );
    assert!(section_ids(&some).contains(&"custom-resources"));
    let kinds: Vec<_> = some
        .iter()
        .filter_map(|r| match r {
            Row::Entry(e) if e.depth == 2 => Some((&*e.id, e.target.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        [(
            "crd:argoproj.io/applications",
            Some(SidebarTarget::kind("argoproj.io", "applications"))
        )]
    );
    // Nothing listable (or no CRDs at all, or discovery not back yet): the section is gone.
    for (rules, custom) in [
        (can_list(&[("", "pods")]), Some(argo())),
        (AccessRules::all_access(), Some(Vec::new())),
        (AccessRules::all_access(), None),
    ] {
        let rows = rows_for(&known(rules), custom.as_deref(), &BTreeMap::new(), &[]);
        assert!(!section_ids(&rows).contains(&"custom-resources"));
    }
}

#[test]
fn crd_groups_start_closed_and_sections_start_open() {
    let rows = rows_for(
        &known(AccessRules::all_access()),
        Some(&argo()),
        &BTreeMap::new(),
        &[],
    );
    let Some(Row::Group(group)) = rows.iter().find(|r| matches!(r, Row::Group(_))) else {
        panic!("the group row is there");
    };
    assert!(!group.open);
    assert!(
        rows.iter().any(|r| r.id() == "workloads/pods"),
        "sections list their entries by default"
    );
}

#[test]
fn a_closed_section_keeps_its_heading_and_drops_its_entries() {
    let mut open = BTreeMap::new();
    open.insert("workloads".to_owned(), false);
    let rows = rows_for(&known(AccessRules::all_access()), None, &open, &[]);
    let Some(Row::Section(workloads)) = rows.iter().find(|r| r.id() == "workloads") else {
        panic!("the heading stays");
    };
    assert!(!workloads.open && workloads.expandable);
    assert!(!rows.iter().any(|r| r.id().starts_with("workloads/")));
}

#[test]
fn before_the_review_answers_only_sections_needing_nothing_show() {
    let rows = rows_for(&AccessState::Pending, Some(&argo()), &BTreeMap::new(), &[]);
    assert_eq!(section_ids(&rows), ["cluster"]);
    let Some(Row::Notice(notice)) = rows.last() else {
        panic!("a checking line closes the list");
    };
    assert_eq!(notice.id, "access-pending");
}

#[test]
fn a_failed_review_shows_everything_and_warns_on_top() {
    let access = AccessState::Known(AccessOutcome::Failed {
        reason: "connection reset".into(),
    });
    let rows = rows_for(&access, Some(&argo()), &BTreeMap::new(), &[]);
    assert_eq!(section_ids(&rows).len(), 11);
    let Some(Row::Notice(warning)) = rows.first() else {
        panic!("the warning is the first row");
    };
    assert_eq!(warning.kind, NoticeKind::Warning);
    assert!(warning.text.contains("connection reset"));
    assert!(!rows.iter().any(|r| r.id() == "access-limited"));
}

fn integration(id: &str, section: &str, items: &[&str]) -> IntegrationSection {
    IntegrationSection {
        integration: id.into(),
        section: PortSection {
            id: section.into(),
            title: section.to_uppercase(),
            icon: None,
            items: items
                .iter()
                .map(|item| SidebarItem {
                    id: (*item).into(),
                    title: (*item).into(),
                    icon: None,
                    command: CommandId::new("argo::Open"),
                    needs: Default::default(),
                })
                .collect(),
        },
    }
}

#[test]
fn integration_sections_come_after_the_core_ones_in_order() {
    let integrations = [
        integration("argocd", "apps", &["list"]),
        integration("flux", "kustomizations", &["list", "tree"]),
    ];
    let rows = rows_for(
        &known(AccessRules::all_access()),
        Some(&argo()),
        &BTreeMap::new(),
        &integrations,
    );
    let sections = section_ids(&rows);
    let core = sections.len() - 2;
    assert_eq!(
        sections[core..],
        ["integration:argocd/apps", "integration:flux/kustomizations"]
    );
    assert_eq!(
        sections[core - 1],
        "custom-resources",
        "after the core ones"
    );
    // Their items dispatch their command.
    let Some(Row::Entry(item)) = rows
        .iter()
        .find(|r| r.id() == "integration:argocd/apps/list")
    else {
        panic!("the integration's item is listed");
    };
    assert_eq!(
        item.target,
        Some(SidebarTarget::Command(CommandId::new("argo::Open")))
    );
}

#[test]
fn row_ids_are_unique() {
    let mut open = BTreeMap::new();
    open.insert("crd:argoproj.io".to_owned(), true);
    let rows = rows_for(
        &known(AccessRules::all_access()),
        Some(&argo()),
        &open,
        &[integration("argocd", "apps", &["list"])],
    );
    let mut ids: Vec<_> = rows.iter().map(|r| r.id().to_owned()).collect();
    let total = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), total);
}

#[test]
fn custom_groups_carry_how_many_kinds_they_have_and_the_crd_list_comes_first() {
    let rows = rows_for(
        &known(AccessRules::all_access()),
        Some(&argo()),
        &BTreeMap::new(),
        &[],
    );
    let ids: Vec<&str> = rows.iter().map(Row::id).collect();
    let at = ids.iter().position(|id| *id == "custom-resources").unwrap();
    assert_eq!(
        &ids[at..],
        [
            "custom-resources",
            "custom-resources/definitions",
            "crd:argoproj.io"
        ]
    );
    let Some(Row::Group(group)) = rows.iter().find(|r| matches!(r, Row::Group(_))) else {
        panic!("a group row");
    };
    assert_eq!(group.count, Some(2), "AppProject and Application");
    let Some(Row::Entry(definitions)) = rows
        .iter()
        .find(|r| r.id() == "custom-resources/definitions")
    else {
        panic!("the definitions entry");
    };
    assert_eq!(
        definitions.target,
        Some(SidebarTarget::Command(CommandId::CRD_OPEN_LIST))
    );
    assert_eq!(definitions.depth, 1);
}

#[test]
fn the_definitions_entry_needs_list_on_customresourcedefinitions() {
    // The user may list one custom kind but not the definitions: the group shows, the entry not.
    let rows = rows_for(
        &known(can_list(&[("argoproj.io", "applications")])),
        Some(&argo()),
        &BTreeMap::new(),
        &[],
    );
    assert!(rows.iter().any(|r| r.id() == "crd:argoproj.io"));
    assert!(
        !rows
            .iter()
            .any(|r| r.id() == "custom-resources/definitions")
    );
    let both = rows_for(
        &known(can_list(&[
            ("argoproj.io", "applications"),
            ("apiextensions.k8s.io", "customresourcedefinitions"),
        ])),
        Some(&argo()),
        &BTreeMap::new(),
        &[],
    );
    assert!(
        both.iter()
            .any(|r| r.id() == "custom-resources/definitions")
    );
}
