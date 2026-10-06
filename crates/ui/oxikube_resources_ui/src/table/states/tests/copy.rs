//! The words: each state has its own, names the verb and resource where it can, and never shows
//! an unredacted secret.

use oxikube_domain::ErrorKind;
use oxikube_domain::session::WatchScope;
use oxikube_ui::IconName;

use crate::table::states::copy::{copy, scope_label, short};
use crate::table::states::{StateLabels, TableState};

fn labels() -> StateLabels {
    StateLabels::new("deployments", "deployments.apps", "namespace “shop”")
}

#[test]
fn every_state_has_its_own_icon_and_title() {
    let l = labels();
    let states = [
        TableState::Loading,
        TableState::Empty,
        TableState::FilteredEmpty {
            filter: "web".into(),
        },
        TableState::Forbidden {
            message: "no".into(),
        },
        TableState::Unauthorized {
            message: "expired".into(),
        },
        TableState::Failed {
            kind: ErrorKind::Timeout,
            message: "slow".into(),
        },
    ];
    let titles: Vec<String> = states.iter().map(|s| copy(s, &l).title).collect();
    let mut unique = titles.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), titles.len(), "{titles:?}");
    let icons: Vec<IconName> = states.iter().map(|s| copy(s, &l).icon).collect();
    assert_eq!(
        icons,
        [
            IconName::LoaderCircle,
            IconName::Box,
            IconName::Funnel,
            IconName::Lock,
            IconName::KeyRound,
            IconName::CircleAlert
        ]
    );
}

#[test]
fn empty_says_which_namespace_and_filtered_empty_says_the_filter() {
    let l = labels();
    assert_eq!(
        copy(&TableState::Empty, &l).title,
        "No deployments in namespace “shop”"
    );
    let filtered = copy(
        &TableState::FilteredEmpty {
            filter: "web-".into(),
        },
        &l,
    );
    assert_eq!(filtered.title, "No deployments match “web-”");
    assert!(filtered.hint.unwrap().contains("namespace “shop”"));
}

#[test]
fn forbidden_names_the_verb_and_the_resource_and_points_at_rbac() {
    let words = copy(
        &TableState::Forbidden {
            message: "deployments.apps is forbidden".into(),
        },
        &labels(),
    );
    let hint = words.hint.expect("a hint");
    assert!(hint.contains("`list`"), "{hint}");
    assert!(hint.contains("`deployments.apps`"), "{hint}");
    assert!(hint.contains("RBAC"), "{hint}");
}

#[test]
fn unauthorized_reads_as_an_auth_problem_not_an_unknown_resource() {
    let words = copy(
        &TableState::Unauthorized {
            message: "the server could not find the requested resource".into(),
        },
        &labels(),
    );
    assert_eq!(words.title, "Your credentials were rejected");
    let hint = words.hint.expect("a hint");
    assert!(hint.contains("expired"), "{hint}");
    assert!(
        !words.title.to_lowercase().contains("unknown")
            && !words.title.to_lowercase().contains("not found")
    );
}

#[test]
fn an_error_has_a_short_line_and_the_rest_behind_details() {
    let long = format!("first line\n{}", "more detail ".repeat(40));
    let words = copy(
        &TableState::Failed {
            kind: ErrorKind::Network,
            message: long,
        },
        &labels(),
    );
    assert_eq!(words.summary.as_deref(), Some("first line"));
    assert!(words.detail.expect("details").contains("more detail"));
    let one_liner = copy(
        &TableState::Failed {
            kind: ErrorKind::Internal,
            message: "boom".into(),
        },
        &labels(),
    );
    assert_eq!(one_liner.summary.as_deref(), Some("boom"));
    assert_eq!(one_liner.detail, None, "nothing more to show");
}

#[test]
fn server_text_is_redacted_before_it_is_shown() {
    let words = copy(
        &TableState::Failed {
            kind: ErrorKind::Network,
            message: "dial https://user:hunter2@api.example.com:6443 failed".into(),
        },
        &labels(),
    );
    let shown = format!("{:?}", words);
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(!short("Bearer abcdefghijklmnopqrstuvwxyz0123456789ABCDEF").contains("abcdefghijkl"));
}

#[test]
fn long_text_is_bounded() {
    let text = "x".repeat(5000);
    assert!(short(&text).chars().count() <= 161);
}

#[test]
fn the_scope_reads_naturally() {
    assert_eq!(scope_label(&WatchScope::Cluster, true), "all namespaces");
    assert_eq!(scope_label(&WatchScope::Cluster, false), "the cluster");
    assert_eq!(
        scope_label(&WatchScope::Namespaces(vec!["a".into()]), true),
        "namespace “a”"
    );
    assert_eq!(
        scope_label(
            &WatchScope::Namespaces(vec!["a".into(), "b".into(), "c".into(), "d".into()]),
            true
        ),
        "namespaces a, b, c and 1 more"
    );
}
