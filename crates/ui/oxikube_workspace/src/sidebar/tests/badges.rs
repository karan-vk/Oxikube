//! Which entries get a count badge, writing the store's answers onto rows, and the badge text.
//! Pure: no window.

use std::collections::{BTreeMap, HashMap};

use oxikube_app::{CountState, KindCount};
use oxikube_domain::access::{AccessRule, AccessRules};

use crate::sidebar::{
    AccessState, KindKey, Row, RowInputs, SidebarEntry, SidebarSection, apply_counts, badge_text,
    build_rows, core_sections, count_plan,
};
use oxikube_app::AccessOutcome;
use oxikube_ui::IconName;

fn key(group: &str, resource: &str) -> KindKey {
    (group.to_owned().into(), resource.to_owned().into())
}

fn counted(total: usize, rated: usize, healthy: usize) -> CountState {
    CountState::Counted(KindCount {
        total,
        rated,
        healthy,
    })
}

fn rows_with(access: &AccessState) -> Vec<Row> {
    let sections = core_sections();
    build_rows(&RowInputs {
        sections: &sections,
        integrations: &[],
        custom: None,
        crd_watch_forbidden: false,
        access,
        open: &BTreeMap::new(),
    })
}

fn all_access() -> AccessState {
    AccessState::Known(AccessOutcome::Reviewed(AccessRules::all_access()))
}

#[test]
fn the_plan_names_every_builtin_kind_the_user_may_list_once() {
    let plan = count_plan(&core_sections(), &all_access(), None);
    let kinds: Vec<&KindKey> = plan.kinds.iter().map(|(k, _)| k).collect();
    assert!(kinds.contains(&&key("", "pods")));
    assert!(kinds.contains(&&key("apps", "deployments")));
    assert!(kinds.contains(&&key("", "nodes")));
    // `events` is one section heading, not an entry, and still has a badge.
    assert!(kinds.contains(&&key("", "events")));
    let mut unique = kinds.clone();
    unique.dedup();
    assert_eq!(unique.len(), kinds.len(), "each kind once");
    assert_eq!(plan.targets().len(), kinds.len());
}

#[test]
fn a_kind_the_user_may_not_list_is_not_counted() {
    let rules =
        AccessRules::none().with_rule(AccessRule::granting(&["list"], &[""], &["pods"], &[]));
    let access = AccessState::Known(AccessOutcome::Reviewed(rules));
    let plan = count_plan(&core_sections(), &access, None);
    let kinds: Vec<&KindKey> = plan.kinds.iter().map(|(k, _)| k).collect();
    assert_eq!(kinds, [&key("", "pods")]);
    // Until the review answers only entries that need nothing are offered: none are counted.
    assert!(
        count_plan(&core_sections(), &AccessState::Pending, None)
            .kinds
            .is_empty()
    );
}

#[test]
fn custom_resources_and_unknown_kinds_have_no_badge() {
    let section =
        SidebarSection::new("argo", "Argo", IconName::Plug, 1).with_entries([SidebarEntry::kind(
            "apps",
            "Applications",
            "argoproj.io",
            "applications",
        )]);
    let plan = count_plan(
        &[section],
        &AccessState::Known(AccessOutcome::Failed {
            reason: String::new(),
        }),
        None,
    );
    assert!(plan.kinds.is_empty());
}

#[test]
fn single_kind_sections_show_their_kind_total() {
    let plan = count_plan(&core_sections(), &all_access(), None);
    let sections: Vec<&str> = plan.sections.iter().map(|(id, _)| &**id).collect();
    for id in ["nodes", "namespaces", "events"] {
        assert!(sections.contains(&id), "{id}");
    }
    assert!(!sections.contains(&"workloads"), "workloads has many kinds");
}

#[test]
fn answers_land_on_the_entries_and_single_kind_sections() {
    let access = all_access();
    let plan = count_plan(&core_sections(), &access, None);
    let mut rows = rows_with(&access);
    let states: HashMap<KindKey, CountState> = [
        (key("", "pods"), counted(5, 5, 4)),
        (key("", "nodes"), counted(3, 3, 3)),
        (
            key("apps", "deployments"),
            CountState::NoAccess {
                message: "denied".into(),
            },
        ),
    ]
    .into();
    apply_counts(&mut rows, &plan, &states);
    let entry = |rows: &[Row], id: &str| match rows.iter().find(|r| r.id() == id) {
        Some(Row::Entry(e)) => e.count.clone(),
        other => panic!("{id}: {other:?}"),
    };
    assert_eq!(entry(&rows, "workloads/pods"), Some(counted(5, 5, 4)));
    assert!(matches!(
        entry(&rows, "workloads/deployments"),
        Some(CountState::NoAccess { .. })
    ));
    assert_eq!(entry(&rows, "workloads/jobs"), None, "no answer, no badge");
    let section_count = |rows: &[Row], id: &str| match rows.iter().find(|r| r.id() == id) {
        Some(Row::Section(s)) => s.count.as_ref().and_then(|c| c.count()).map(|c| c.total),
        other => panic!("{id}: {other:?}"),
    };
    assert_eq!(section_count(&rows, "nodes"), Some(3));
    assert_eq!(section_count(&rows, "workloads"), None);
    // Clearing the answers clears the badges.
    apply_counts(&mut rows, &plan, &HashMap::new());
    assert_eq!(entry(&rows, "workloads/pods"), None);
    assert_eq!(section_count(&rows, "nodes"), None);
}

#[test]
fn a_forbidden_single_kind_section_says_no_access() {
    let access = all_access();
    let plan = count_plan(&core_sections(), &access, None);
    let mut rows = rows_with(&access);
    let states: HashMap<KindKey, CountState> = [(
        key("", "nodes"),
        CountState::NoAccess {
            message: "nodes is forbidden".into(),
        },
    )]
    .into();
    apply_counts(&mut rows, &plan, &states);
    let Some(Row::Section(nodes)) = rows.iter().find(|r| r.id() == "nodes") else {
        panic!("no nodes section");
    };
    let state = nodes.count.as_ref().expect("the section carries the state");
    assert!(state.is_no_access());
    let (text, hover) = badge_text(state).expect("a badge");
    assert_eq!(text, "no access", "not the not-loaded dash");
    assert_eq!(hover.as_deref(), Some("nodes is forbidden"));
}

#[test]
fn badge_text_says_what_the_number_means() {
    let (text, hover) = badge_text(&counted(12, 12, 10)).expect("a badge");
    assert_eq!(text, "12");
    assert_eq!(hover.as_deref(), Some("10 healthy, 2 not"));
    let (text, hover) = badge_text(&counted(4, 0, 0)).expect("a badge");
    assert_eq!(
        (text.as_str(), hover),
        ("4", None),
        "no rule, no health claim"
    );
    let (text, _) = badge_text(&CountState::NoAccess {
        message: String::new(),
    })
    .expect("a badge");
    assert_eq!(text, "no access", "never a zero");
    let (text, hover) = badge_text(&CountState::OverBudget {
        message: "watch budget: 8 feeds already open".into(),
    })
    .expect("a badge");
    assert_eq!(text, "–");
    assert!(hover.is_some_and(|h| h.contains("watch budget")));
    let (text, _) = badge_text(&CountState::Loading).expect("a badge");
    assert_eq!(text, "…");
    assert_eq!(badge_text(&CountState::NotWatched), None);
}
