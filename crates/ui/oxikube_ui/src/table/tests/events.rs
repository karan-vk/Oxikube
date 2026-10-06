//! Row clicks with modifiers, and tables whose owner selects.

use super::support::harness;
use crate::table::{RowClick, TableEvent};
use gpui::{Modifiers, TestAppContext, VisualTestContext};
use std::cell::RefCell;
use std::rc::Rc;

fn click_first_cell(cx: &mut VisualTestContext, modifiers: Modifiers) {
    let at = cx
        .debug_bounds("td-0-0")
        .expect("row 0 was laid out")
        .center();
    cx.simulate_click(at, modifiers);
    cx.run_until_parked();
}

#[gpui::test]
fn row_clicks_carry_their_modifiers_before_the_selection(cx: &mut TestAppContext) {
    let (view, cx) = harness(cx, 20);
    cx.run_until_parked();
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    let table = view.read_with(cx, |h, _| h.table.clone());
    let _subscription = cx
        .update(|_, cx| table.on_event(cx, move |event, _| sink.borrow_mut().push(event.clone())));

    click_first_cell(cx, Modifiers::secondary_key());
    let seen = events.borrow().clone();
    assert_eq!(
        seen.first(),
        Some(&TableEvent::RowClicked(RowClick {
            row: 0,
            extend: false,
            toggle: true,
            count: 1,
        }))
    );
    assert!(
        seen.contains(&TableEvent::SelectRow(0)),
        "a table that selects rows still selects: {seen:?}"
    );

    events.borrow_mut().clear();
    click_first_cell(cx, Modifiers::shift());
    assert!(matches!(
        events.borrow().first(),
        Some(TableEvent::RowClicked(RowClick {
            extend: true,
            toggle: false,
            ..
        }))
    ));
}
