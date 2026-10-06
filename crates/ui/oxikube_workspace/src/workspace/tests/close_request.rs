//! Items that ask before they close ([`Item::intercepts_close`]): their own close button,
//! `workspace::CloseActiveItem`, and the plain close that bypasses the question.

use gpui::{Bounds, Pixels, Point, TestAppContext, point};

use super::*;

fn centre(bounds: Bounds<Pixels>) -> Point<Pixels> {
    point(
        bounds.origin.x + bounds.size.width / 2.,
        bounds.origin.y + bounds.size.height / 2.,
    )
}

fn open_intercepting(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    title: &str,
    deferred: bool,
) -> (EntityId, Entity<TestItem>) {
    let title = title.to_owned();
    let item = vcx.update(|_, cx| cx.new(|cx| TestItem::new(title, cx).intercepting(deferred)));
    let id = vcx.update(|window, cx| {
        let item = item.clone();
        ws.update(cx, |ws, cx| ws.open_item(item, window, cx))
    });
    vcx.run_until_parked();
    (id, item)
}

#[gpui::test]
fn an_intercepting_item_draws_its_own_close_button_in_place_of_the_docks(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "plain");
    open_intercepting(&ws, &mut vcx, "asks", true);
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("tab-close-asks").is_some());
    assert!(
        vcx.debug_bounds("tab-close-plain").is_none(),
        "a plain item keeps the dock's close button"
    );
}

#[gpui::test]
fn the_close_button_asks_the_item_and_a_deferred_answer_keeps_the_tab(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let (id, item) = open_intercepting(&ws, &mut vcx, "asks", true);
    let bounds = bounds(&mut vcx, "tab-close-asks").expect("the button");
    vcx.simulate_click(centre(bounds), Default::default());
    vcx.run_until_parked();

    assert_eq!(vcx.update(|_, cx| item.read(cx).close_requests.get()), 1);
    assert!(
        vcx.update(|_, cx| ws.read(cx).item(id).is_some()),
        "the tab stayed"
    );
    assert_eq!(
        vcx.update(|_, cx| item.read(cx).closed.get()),
        0,
        "and was not closed"
    );
}

#[gpui::test]
fn an_answer_to_close_closes_the_tab(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let (id, item) = open_intercepting(&ws, &mut vcx, "asks", false);
    let bounds = bounds(&mut vcx, "tab-close-asks").expect("the button");
    vcx.simulate_click(centre(bounds), Default::default());
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| item.read(cx).close_requests.get()), 1);
    assert!(vcx.update(|_, cx| ws.read(cx).item(id).is_none()));
    assert_eq!(vcx.update(|_, cx| item.read(cx).closed.get()), 1);
}

#[gpui::test]
fn close_active_item_asks_and_a_forced_close_does_not(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let (id, item) = open_intercepting(&ws, &mut vcx, "asks", true);

    let handled = vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.close_active_item(window, cx)));
    assert!(handled, "being asked counts as handled");
    assert_eq!(vcx.update(|_, cx| item.read(cx).close_requests.get()), 1);
    assert!(vcx.update(|_, cx| ws.read(cx).item(id).is_some()));

    // The item closing itself, or a caller that knows better, bypasses the question.
    let closed = vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.close_item(id, window, cx)));
    assert!(closed);
    assert_eq!(vcx.update(|_, cx| item.read(cx).close_requests.get()), 1);
}

#[gpui::test]
fn an_item_that_cannot_close_is_not_asked(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let item = vcx.update(|_, cx| {
        cx.new(|cx| {
            let mut item = TestItem::new("pinned", cx).intercepting(false);
            item.closable = false;
            item
        })
    });
    let id = vcx.update(|window, cx| {
        let item = item.clone();
        ws.update(cx, |ws, cx| ws.open_item(item, window, cx))
    });
    vcx.run_until_parked();
    assert!(
        !vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.request_close_item(id, window, cx)))
    );
    assert_eq!(vcx.update(|_, cx| item.read(cx).close_requests.get()), 0);
}
