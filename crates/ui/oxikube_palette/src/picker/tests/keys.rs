// Portions of this file are derived from Zed (https://github.com/zed-industries/zed),
// Copyright (c) Zed Industries, Inc. and contributors.
// Zed is licensed under the GNU General Public License v3.0 or later.
// Modifications Copyright (c) Oxikube contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Source: crates/picker/src/picker.rs @ zed a84689073d296dfd39987bc7dd478e43ef76d83a

//! Keystrokes through the shipped keymap: navigation, confirm, secondary confirm, dismissal.

use gpui::{Modifiers, TestAppContext, point, px};

use super::{Fixture, secondary_enter};
use crate::picker::test_support::TestDelegate;

#[gpui::test]
fn the_arrow_home_and_end_keys_move_the_selection_and_wrap(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open(TestDelegate::numbered(5_000));
    assert_eq!(f.read(|p| p.delegate.match_texts().len()), 5_000);
    assert_eq!(f.selected().as_deref(), Some("item-0000"));

    f.keys("down down");
    assert_eq!(f.selected().as_deref(), Some("item-0002"));
    f.keys("up");
    assert_eq!(f.selected().as_deref(), Some("item-0001"));
    f.keys("end");
    assert_eq!(f.selected().as_deref(), Some("item-4999"));
    f.keys("down");
    assert_eq!(f.selected().as_deref(), Some("item-0000"), "down wraps");
    f.keys("up");
    assert_eq!(f.selected().as_deref(), Some("item-4999"), "up wraps");
    f.keys("home");
    assert_eq!(f.selected().as_deref(), Some("item-0000"));
    f.keys("pagedown");
    assert_eq!(f.selected().as_deref(), Some("item-4999"));
    f.keys("pageup ctrl-n ctrl-n ctrl-p");
    assert_eq!(f.selected().as_deref(), Some("item-0001"));
}

#[gpui::test]
fn moving_to_the_end_scrolls_the_selection_into_view(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open(TestDelegate::numbered(5_000));
    assert!(f.vcx.debug_bounds("test-item-0").is_some());
    f.keys("end");
    assert!(f.vcx.debug_bounds("test-item-4999").is_some(), "revealed");
    assert!(f.vcx.debug_bounds("test-item-0").is_none(), "scrolled away");
}

#[gpui::test]
fn keyboard_selection_skips_matches_that_cannot_be_selected(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open(TestDelegate::new(["a", "header", "c", "d"]).unselectable([1]));
    f.keys("down");
    assert_eq!(f.selected().as_deref(), Some("c"), "skips the header");
    f.keys("up");
    assert_eq!(f.selected().as_deref(), Some("a"));
    // A click on it does nothing either.
    let record = f.read(|p| p.delegate.record());
    f.click("test-item-1", Modifiers::none());
    assert!(record.confirmed.borrow().is_empty());
    assert!(f.picker().is_some(), "still open");
}

#[gpui::test]
fn enter_confirms_the_selection_and_closes_the_picker(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let record = f.open(TestDelegate::numbered(5_000));
    f.keys("down down enter");
    assert_eq!(
        *record.confirmed.borrow(),
        [("item-0002".to_owned(), false)]
    );
    assert!(f.picker().is_none(), "closed");
    assert_eq!(record.dismissed.get(), 1, "told once that it closes");
    assert!(f.item_has_focus(), "the focus is back where it was");
}

#[gpui::test]
fn the_secondary_confirm_key_takes_the_secondary_path(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let record = f.open(TestDelegate::numbered(10));
    f.keys("down");
    f.keys(secondary_enter());
    assert_eq!(*record.confirmed.borrow(), [("item-0001".to_owned(), true)]);
}

#[gpui::test]
fn escape_dismisses_without_confirming_and_returns_the_focus(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let record = f.open(TestDelegate::numbered(5_000));
    assert!(!f.item_has_focus(), "the query field has it");
    f.type_text("item");
    f.keys("escape");
    assert!(f.picker().is_none(), "closed");
    assert!(record.confirmed.borrow().is_empty());
    assert_eq!(record.dismissed.get(), 1);
    assert!(f.item_has_focus(), "the focus is back where it was");
}

#[gpui::test]
fn a_click_confirms_that_row_and_a_platform_click_is_secondary(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let record = f.open(TestDelegate::numbered(10));
    f.click("picker-row-3", Modifiers::none());
    assert_eq!(
        *record.confirmed.borrow(),
        [("item-0003".to_owned(), false)]
    );
    assert!(f.picker().is_none());

    let record = f.open(TestDelegate::numbered(10));
    f.click("picker-row-2", Modifiers::secondary_key());
    assert_eq!(*record.confirmed.borrow(), [("item-0002".to_owned(), true)]);
}

#[gpui::test]
fn a_click_outside_dismisses_and_tells_the_delegate_once(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let record = f.open(TestDelegate::numbered(10));
    f.vcx
        .simulate_click(point(px(2.), px(2.)), Modifiers::none());
    f.settle();
    assert!(f.picker().is_none(), "closed");
    assert_eq!(record.dismissed.get(), 1);
    assert!(record.confirmed.borrow().is_empty());
    assert!(f.item_has_focus());
}
