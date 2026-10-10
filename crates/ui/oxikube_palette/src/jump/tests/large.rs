//! A cluster with more kinds than the inline limit (CRD-heavy): the completions of a typed word
//! are matched on the background executor, and the bar stays correct while they are pending.

use crate::picker::PickerDelegate as _;
use gpui::TestAppContext;
use oxikube_testkit::kinds::{KindSpec, core_kinds};

use super::{Fixture, wait_for_data};
use crate::picker::fuzzy::INLINE_MATCH_LIMIT;

fn crd_heavy() -> Vec<oxikube_domain::kinds::ResourceKind> {
    let mut kinds = core_kinds();
    kinds.extend((0..INLINE_MATCH_LIMIT + 200).map(|i| {
        KindSpec::new(
            &format!("group{}.example.io", i % 40),
            "v1",
            &format!("Widget{i}"),
            &format!("widgets{i}"),
        )
        .short(&format!("wd{i}"))
        .build()
    }));
    kinds
}

/// Opens the bar on a CRD-heavy `dev` and lets the data land.
fn open_large(cx: &mut TestAppContext) -> Fixture {
    let mut f = Fixture::with_kinds(cx, crd_heavy());
    f.open();
    wait_for_data(&mut f);
    f
}

/// Sets the query the way typing does, without letting the background match finish.
fn set_query_unsettled(f: &mut Fixture, text: &str) {
    let picker = f.picker();
    f.vcx
        .update(|window, cx| picker.update(cx, |p, cx| p.set_query(text, window, cx)));
}

#[gpui::test]
fn the_pool_is_above_the_inline_limit(cx: &mut TestAppContext) {
    let mut f = open_large(cx);
    // A blank prefix lists in order, inline: the whole pool is there to be matched.
    assert!(f.read(|d| d.completions().len()) > INLINE_MATCH_LIMIT);
}

#[gpui::test]
fn a_typed_prefix_is_matched_in_the_background_and_then_applied(cx: &mut TestAppContext) {
    let mut f = open_large(cx);
    let before = f.read(|d| d.completions());
    set_query_unsettled(&mut f, "widgets77");
    let picker = f.picker();
    assert!(
        f.vcx.update(|_, cx| picker.read(cx).is_matching()),
        "the match is off the UI thread, still pending"
    );
    assert_eq!(
        f.read(|d| d.completions()),
        before,
        "the previous matches stay until the new ones land"
    );

    f.settle();
    assert!(!f.vcx.update(|_, cx| picker.read(cx).is_matching()));
    let listed = f.read(|d| d.completions());
    assert_eq!(listed.first().map(String::as_str), Some("widgets77"));
    assert!(listed.len() < before.len());
    assert_eq!(
        f.read(|d| d.completion().map(|c| c.2.to_string())),
        Some("widgets77".into())
    );
}

#[gpui::test]
fn the_selection_goes_back_to_the_top_when_the_new_matches_land(cx: &mut TestAppContext) {
    let mut f = open_large(cx);
    f.type_text("widgets7");
    f.keys("down");
    f.keys("down");
    assert!(f.read(|d| d.selected_index()) > 0);
    set_query_unsettled(&mut f, "widgets70");
    f.settle();
    assert_eq!(f.read(|d| d.selected_index()), 0);
}

#[gpui::test]
fn tab_while_matching_completes_from_the_newest_line_not_the_previous_one(cx: &mut TestAppContext) {
    let mut f = open_large(cx);
    f.type_text("widgets1");
    set_query_unsettled(&mut f, "widgets77");
    let bar = f.bar().expect("open");
    f.vcx.update(|window, cx| {
        bar.update(cx, |bar, cx| bar.complete(window, cx));
    });
    // Nothing completed from the stale list yet.
    assert_eq!(f.query(), "widgets77");

    f.settle();
    assert_eq!(f.query(), "widgets77 ", "completed from the newest matches");
}

#[gpui::test]
fn a_query_that_goes_back_to_the_inline_path_drops_a_waiting_tab(cx: &mut TestAppContext) {
    let mut f = open_large(cx);
    set_query_unsettled(&mut f, "widgets77");
    let bar = f.bar().expect("open");
    f.vcx.update(|window, cx| {
        bar.update(cx, |bar, cx| bar.complete(window, cx));
    });
    // A blank line matches inline: the Tab that waited for the old line is not replayed on it.
    set_query_unsettled(&mut f, "");
    f.settle();
    assert_eq!(f.query(), "");
    // ... nor on the next line that is matched in the background.
    f.type_text("widgets5");
    assert_eq!(f.query(), "widgets5");
}
