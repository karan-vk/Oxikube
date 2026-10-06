//! Searching: typing narrows the list, ranks it, and clearing brings it back.

use gpui::TestAppContext;

use super::{Fixture, named};

fn clusters() -> Vec<oxikube_ports::ClusterContext> {
    named(&["prod-eu", "prod-us", "staging-eu", "dev-local", "qa"])
}

#[gpui::test]
fn the_search_field_has_the_focus_when_the_catalog_opens(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, clusters());
    f.type_text("prod");
    assert_eq!(
        f.names(),
        ["prod-eu", "prod-us"],
        "typing needs no click first"
    );
}

#[gpui::test]
fn typing_narrows_the_list_as_you_type_and_clearing_restores_it(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, clusters());
    assert_eq!(f.names().len(), 5);

    f.type_text("p");
    assert_eq!(f.names(), ["prod-eu", "prod-us"]);
    f.type_text("rod-us");
    assert_eq!(f.names()[0], "prod-us", "the closest match comes first");

    f.type_text("x");
    assert!(f.names().is_empty(), "no row has all of those letters");
    f.window.draw_frame();
    assert!(
        !f.is_laid_out("catalog-row-0"),
        "rows that do not match are not rendered"
    );
    f.keys("backspace");

    // Escape clears the field (and so the filter).
    f.keys("escape");
    assert_eq!(f.names().len(), 5);
}

#[gpui::test]
fn a_search_that_matches_nothing_says_so_and_a_clear_brings_the_list_back(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, clusters());
    f.type_text("zzzz");
    f.window.draw_frame();
    assert!(f.is_laid_out("catalog-no-match"));
    assert!(
        !f.is_laid_out("catalog-empty"),
        "the catalog is not empty, the search is"
    );
    assert!(f.names().is_empty());
    f.keys("escape");
    f.window.draw_frame();
    assert!(!f.is_laid_out("catalog-no-match"));
    assert!(f.is_laid_out("catalog-row-4"));
}

#[gpui::test]
fn the_search_matches_loosely_not_just_by_prefix(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, clusters());
    f.type_text("stgeu");
    assert_eq!(f.names(), ["staging-eu"], "a fuzzy match skips letters");
}

#[gpui::test]
fn searching_looks_at_the_user_and_source_columns_too(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, clusters());
    f.type_text("qa-user");
    assert_eq!(f.names(), ["qa"]);
}

#[gpui::test]
fn the_best_match_is_selected_so_enter_connects_it(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, clusters());
    f.type_text("qa");
    assert_eq!(
        f.read(|v| v.model().selected_entry().map(|e| e.name().to_owned())),
        Some("qa".into())
    );
    f.keys("enter");
    assert_eq!(f.recorder.sent().len(), 1);
}

#[gpui::test]
fn a_reload_keeps_the_search_and_the_selection(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, clusters());
    f.type_text("eu");
    f.keys("down");
    let selected = f.read(|v| v.model().selected().cloned());
    f.source.set_contexts(named(&[
        "prod-eu",
        "prod-us",
        "staging-eu",
        "dev-local",
        "qa",
        "new-eu",
    ]));
    f.app.run_until_parked();
    assert_eq!(f.read(|v| v.model().query().to_owned()), "eu");
    assert!(f.names().contains(&"new-eu".to_owned()));
    assert_eq!(f.read(|v| v.model().selected().cloned()), selected);
}
