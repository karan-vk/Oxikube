// Portions of this file are derived from Zed (https://github.com/zed-industries/zed),
// Copyright (c) Zed Industries, Inc. and contributors.
// Zed is licensed under the GNU General Public License v3.0 or later.
// Modifications Copyright (c) Oxikube contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Source: crates/picker/src/picker.rs @ zed a84689073d296dfd39987bc7dd478e43ef76d83a

//! Matching: typing filters, a slow old query never wins, a confirm waits for its matches, the
//! empty state, and only the rows on screen are built.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::TestAppContext;

use super::Fixture;
use crate::picker::fuzzy::{INLINE_MATCH_LIMIT, StringMatchCandidate, match_strings_async};
use crate::picker::test_support::TestDelegate;

#[gpui::test]
fn typing_filters_and_selects_the_best_match(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let record = f.open(TestDelegate::numbered(5_000));
    f.type_text("4999");
    assert_eq!(f.read(|p| p.delegate.match_texts()), ["item-4999"]);
    assert_eq!(f.selected().as_deref(), Some("item-4999"));
    assert_eq!(
        record.queries.borrow().last().map(String::as_str),
        Some("4999")
    );
    f.keys("enter");
    assert_eq!(
        *record.confirmed.borrow(),
        [("item-4999".to_owned(), false)]
    );
}

#[gpui::test]
fn a_small_list_matches_inline_and_a_large_one_off_the_ui_thread(cx: &mut TestAppContext) {
    let list = |n: usize| -> Arc<[StringMatchCandidate]> {
        (0..n)
            .map(|ix| StringMatchCandidate::new(ix, format!("item-{ix:04}")))
            .collect()
    };
    let executor = cx.background_executor.clone();
    let inline = match_strings_async(list(INLINE_MATCH_LIMIT), "0001".into(), 10, &executor);
    assert!(inline.is_ready(), "matched on the calling thread");
    let background = match_strings_async(list(2_000), "1999".into(), 10, &executor);
    assert!(!background.is_ready(), "queued on the background executor");
    let found = Rc::new(RefCell::new(None));
    let sink = found.clone();
    cx.spawn(async move |_| *sink.borrow_mut() = Some(background.await))
        .detach();
    cx.run_until_parked();
    let found = found.borrow_mut().take().expect("the match finished");
    assert_eq!(found[0].string.as_ref(), "item-1999");
}

#[gpui::test]
fn a_slow_old_query_does_not_overwrite_a_newer_one(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open(TestDelegate::numbered(5_000).slow("item-1", Duration::from_millis(100)));
    f.update(|picker, window, cx| picker.set_query("item-1", window, cx));
    f.update(|picker, window, cx| picker.set_query("4999", window, cx));
    f.settle();
    assert_eq!(f.read(|p| p.delegate.match_texts()), ["item-4999"]);

    f.vcx.executor().advance_clock(Duration::from_millis(200));
    f.settle();
    assert_eq!(
        f.read(|p| p.delegate.match_texts()),
        ["item-4999"],
        "the old query's late result never lands"
    );
}

#[gpui::test]
fn a_confirm_while_matching_waits_for_the_newest_matches(cx: &mut TestAppContext) {
    // Zed's `test_refresh_waits_for_latest_matches_before_confirming`, through keys.
    let mut f = Fixture::new(cx);
    let record = f.open(
        TestDelegate::numbered(5_000)
            .slow("item-1", Duration::from_millis(100))
            .slow("4999", Duration::from_millis(50)),
    );
    f.update(|picker, window, cx| picker.set_query("item-1", window, cx));
    f.keys("enter");
    assert!(
        record.confirmed.borrow().is_empty(),
        "waits for the matches"
    );
    f.update(|picker, window, cx| picker.set_query("4999", window, cx));
    assert!(record.confirmed.borrow().is_empty());

    f.vcx.executor().advance_clock(Duration::from_millis(50));
    f.settle();
    assert_eq!(
        *record.confirmed.borrow(),
        [("item-4999".to_owned(), false)],
        "the confirm ran on the newest query's matches"
    );
    f.vcx.executor().advance_clock(Duration::from_millis(100));
    f.settle();
    assert_eq!(record.confirmed.borrow().len(), 1);
}

#[gpui::test]
fn no_match_shows_the_empty_text_and_enter_does_nothing(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let record = f.open(TestDelegate::numbered(100));
    f.type_text("zzz");
    assert_eq!(f.read(|p| p.delegate.match_texts().len()), 0);
    assert!(f.vcx.debug_bounds("picker-empty").is_some());
    f.keys("down up home end enter");
    assert!(record.confirmed.borrow().is_empty());
    assert!(f.picker().is_some(), "still open");
}

#[gpui::test]
fn only_the_rows_on_screen_are_built(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let record = f.open(TestDelegate::numbered(5_000));
    record.rendered.set(0);
    f.vcx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    let built = record.rendered.get();
    // 360 px of list over 28 px rows: 13 visible, plus the one measured for the row height.
    assert!(
        (1..=20).contains(&built),
        "{built} rows built for 5 000 matches"
    );
}
