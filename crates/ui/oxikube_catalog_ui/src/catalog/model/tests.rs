//! View-model tests: order, search, selection, badges, and the 2 000-entry budget.

use std::time::Instant;

use jiff::Timestamp;
use oxikube_domain::session::ClusterSessionState;

use super::{Badge, CatalogModel, LoadState, Tone};
use crate::catalog::test_support::{cluster_id, entry, synthetic_entries};

fn model(names: &[&str]) -> CatalogModel {
    let mut model = CatalogModel::new();
    model.set_entries(names.iter().map(|n| entry(n)).collect());
    model
}

#[test]
fn a_new_model_is_loading_and_empty() {
    let model = CatalogModel::new();
    assert_eq!(model.load_state(), &LoadState::Loading);
    assert_eq!(model.total(), 0);
    assert_eq!(model.selected(), None);
    let mut model = model;
    model.set_entries(vec![]);
    assert_eq!(model.load_state(), &LoadState::Ready);
}

#[test]
fn entries_are_ordered_favourites_then_last_used_then_name() {
    let mut entries = vec![entry("zeta"), entry("alpha"), entry("used"), entry("fav")];
    entries[2].last_used = Some(Timestamp::from_second(1_000).unwrap());
    entries[3].favourite = true;
    let mut model = CatalogModel::new();
    model.set_entries(entries);
    assert_eq!(model.visible_names(), ["fav", "used", "alpha", "zeta"]);
    assert_eq!(
        model.selected(),
        Some(&cluster_id("fav")),
        "the first row is selected"
    );
}

#[test]
fn a_search_narrows_ranks_and_clearing_restores_the_order() {
    let mut model = model(&["prod-eu", "staging-eu", "prod-us", "dev"]);
    assert!(model.set_query("prod"));
    assert_eq!(model.visible_names(), ["prod-eu", "prod-us"]);
    assert!(model.is_searching());
    assert_eq!(model.visible_len(), 2);
    assert_eq!(model.total(), 4);

    // A fuzzy search is loose: a weak match (letters spread over other columns) may follow, but
    // the contiguous matches come first.
    assert!(model.set_query("eu"));
    assert_eq!(model.visible_names()[..2], ["prod-eu", "staging-eu"]);

    assert!(model.set_query("zzz"));
    assert!(model.visible_names().is_empty());
    assert_eq!(model.selected(), None);

    assert!(model.set_query(""));
    assert_eq!(
        model.visible_names(),
        ["dev", "prod-eu", "prod-us", "staging-eu"]
    );
    assert!(!model.set_query(""), "no change, no work");
}

#[test]
fn the_search_looks_at_cluster_user_and_source_too_and_every_word_must_match() {
    let mut a = entry("alpha");
    a.context.user = Some("alice".into());
    a.context.cluster_name = Some("eu-central".into());
    let mut b = entry("beta");
    b.context.user = Some("bob".into());
    b.context.cluster_name = Some("eu-west".into());
    let mut model = CatalogModel::new();
    model.set_entries(vec![a, b]);

    model.set_query("alice");
    assert_eq!(model.visible_names(), ["alpha"], "by user");
    model.set_query("west");
    assert_eq!(model.visible_names(), ["beta"], "by cluster");
    model.set_query("kube");
    assert_eq!(model.visible_names().len(), 2, "by source file");
    model.set_query("eu bob");
    assert_eq!(
        model.visible_names(),
        ["beta"],
        "words match across columns"
    );
}

#[test]
fn a_match_in_the_name_outranks_a_match_elsewhere() {
    let mut elsewhere = entry("aaa");
    elsewhere.context.cluster_name = Some("prod".into());
    let named = entry("prod");
    let mut model = CatalogModel::new();
    model.set_entries(vec![elsewhere, named]);
    model.set_query("prod");
    assert_eq!(model.visible_names(), ["prod", "aaa"]);
}

#[test]
fn equal_scores_keep_the_default_order() {
    let mut favourite = entry("node-b");
    favourite.favourite = true;
    let mut model = CatalogModel::new();
    model.set_entries(vec![entry("node-a"), favourite, entry("node-c")]);
    model.set_query("node");
    assert_eq!(model.visible_names(), ["node-b", "node-a", "node-c"]);
}

#[test]
fn the_selection_follows_its_cluster_through_a_search_and_a_reorder() {
    let mut model = model(&["a", "b", "c"]);
    assert!(model.select(2));
    assert_eq!(model.selected(), Some(&cluster_id("c")));

    model.set_query("c");
    assert_eq!(model.selected(), Some(&cluster_id("c")), "still visible");
    assert_eq!(model.selected_index(), Some(0));

    model.set_query("a");
    assert_eq!(
        model.selected(),
        Some(&cluster_id("a")),
        "fell back to the first row"
    );

    model.set_query("");
    model.select(1);
    assert!(model.set_favourite(&cluster_id("b"), true));
    assert_eq!(model.visible_names(), ["b", "a", "c"]);
    assert_eq!(
        model.selected(),
        Some(&cluster_id("b")),
        "the star moved the row, not the selection"
    );
    assert_eq!(model.selected_index(), Some(0));
}

#[test]
fn moving_the_selection_stops_at_the_ends() {
    let mut model = model(&["a", "b", "c"]);
    assert!(!model.select_by(-1), "already first");
    assert!(model.select_by(1));
    assert!(model.select_by(1));
    assert!(!model.select_by(1), "already last");
    assert_eq!(model.selected_index(), Some(2));
    assert!(model.select_first());
    assert!(model.select_last());
    assert_eq!(model.selected_entry().map(|e| e.name()), Some("c"));

    let mut empty = CatalogModel::new();
    empty.set_entries(vec![]);
    assert!(!empty.select_by(1) && !empty.select_first() && !empty.select_last());
}

#[test]
fn a_favourite_reorders_the_list_and_a_last_used_does_not() {
    let mut model = model(&["a", "b"]);
    assert!(!model.set_favourite(&cluster_id("a"), false), "unchanged");
    assert!(
        !model.set_favourite(&cluster_id("ghost"), true),
        "not in the catalog"
    );
    let at = Timestamp::from_second(5_000).unwrap();
    assert!(model.set_last_used(&cluster_id("b"), at));
    assert!(!model.set_last_used(&cluster_id("b"), at));
    assert!(!model.set_last_used(&cluster_id("ghost"), at));
    assert_eq!(
        model.visible_names(),
        ["a", "b"],
        "a click must not move the row under the pointer"
    );
    assert_eq!(model.row(1).and_then(|r| r.entry().last_used), Some(at));
    // The next read sorts it into place.
    let mut entries: Vec<_> = model.visible_rows().map(|r| r.entry().clone()).collect();
    entries.sort_by(|a, b| a.name().cmp(b.name()));
    model.set_entries(entries);
    assert_eq!(model.visible_names(), ["b", "a"]);
}

#[test]
fn a_cluster_with_no_session_is_disconnected() {
    let mut model = model(&["a"]);
    let id = cluster_id("a");
    assert_eq!(model.state(&id), &ClusterSessionState::Disconnected);
    assert_eq!(model.badge(0).map(|b| b.label), Some("Disconnected"));
    assert!(model.set_state(id.clone(), ClusterSessionState::Ready));
    assert!(!model.set_state(id.clone(), ClusterSessionState::Ready));
    assert_eq!(
        model.badge(0).map(|b| (b.label, b.tone)),
        Some(("Connected", Tone::Success))
    );
    assert!(model.clear_state(&id));
    assert!(!model.clear_state(&id));
    assert_eq!(model.state(&id), &ClusterSessionState::Disconnected);
    assert_eq!(model.badge(5), None);
}

#[test]
fn badges_cover_every_state_and_an_invalid_entry_is_an_error_whatever_the_session_says() {
    let cases = [
        (
            ClusterSessionState::Disconnected,
            "Disconnected",
            Tone::Muted,
        ),
        (ClusterSessionState::Connecting, "Connecting", Tone::Info),
        (
            ClusterSessionState::AuthRequired {
                reason: "token expired".into(),
            },
            "Auth required",
            Tone::Warning,
        ),
        (ClusterSessionState::Ready, "Connected", Tone::Success),
        (ClusterSessionState::Degraded, "Degraded", Tone::Warning),
        (
            ClusterSessionState::Error {
                reason: "connection refused".into(),
            },
            "Error",
            Tone::Error,
        ),
    ];
    for (state, label, tone) in cases {
        let badge = Badge::of(None, &state);
        assert_eq!((badge.label, badge.tone), (label, tone));
        assert_eq!(badge.detail.as_deref(), state.reason());
        let invalid = Badge::of(Some("cluster \"x\" is not defined"), &state);
        assert_eq!((invalid.label, invalid.tone), ("Invalid", Tone::Error));
        assert!(invalid.detail.is_some_and(|d| d.contains("not defined")));
    }
}

#[test]
fn a_failed_load_keeps_what_was_shown() {
    let mut model = model(&["a"]);
    model.set_failed("cannot read ~/.kube/config");
    assert!(matches!(model.load_state(), LoadState::Failed(m) if m.contains("cannot read")));
    assert_eq!(model.visible_names(), ["a"]);
}

/// The palette budget (docs/PERFORMANCE.md): filtering 2 000 entries takes at most 5 ms. The
/// bound is the release figure; an unoptimised test build gets a generous multiple so the test
/// guards against an accidental O(n^2) rather than against a slow debug build.
#[test]
fn sorting_and_filtering_two_thousand_entries_stays_in_the_budget() {
    let budget_ms = if cfg!(debug_assertions) { 150.0 } else { 5.0 };
    let mut model = CatalogModel::new();

    let started = Instant::now();
    model.set_entries(synthetic_entries(2_000));
    let load_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(model.total(), 2_000);

    let mut worst: f64 = 0.0;
    for query in [
        "p",
        "pro",
        "prod-eu",
        "staging us 17",
        "team-2 qa",
        "zzzz",
        "",
    ] {
        let started = Instant::now();
        model.set_query(query);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        worst = worst.max(ms);
        assert!(
            ms <= budget_ms,
            "query {query:?} took {ms:.2} ms (budget {budget_ms} ms)"
        );
    }
    assert!(
        load_ms <= budget_ms * 6.0,
        "sorting and preparing 2 000 entries took {load_ms:.2} ms"
    );
    eprintln!("2 000 entries: set_entries {load_ms:.2} ms, slowest filter {worst:.2} ms");
}
