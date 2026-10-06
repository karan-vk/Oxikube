//! The status bar: left and right item registry, ordering by priority, visibility, removal.

use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::{AppContext as _, Entity, VisualTestContext};

use super::*;
use crate::{
    status_bar::{StatusBar, StatusItemId, StatusSide},
    test_support::TestStatusItem,
};

fn register(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    side: StatusSide,
    priority: i32,
    label: &'static str,
) -> (Entity<TestStatusItem>, StatusItemId) {
    let result = vcx.update(|_, cx| {
        let item = cx.new(|_| TestStatusItem::new(label));
        let id = ws.update(cx, |ws, cx| {
            ws.register_status_item(side, priority, item.clone(), cx)
        });
        (item, id)
    });
    vcx.run_until_parked();
    result
}

fn order(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    side: StatusSide,
) -> Vec<StatusItemId> {
    vcx.update(|_, cx| ws.read(cx).status_bar().read(cx).items(side))
}

fn left_edge(vcx: &mut VisualTestContext, label: &str) -> f32 {
    let item = bounds_named(vcx, format!("status-{label}")).expect("item is drawn");
    f32::from(item.origin.x)
}

#[gpui::test]
fn items_order_by_priority_within_each_side(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let (_, c) = register(&ws, &mut vcx, StatusSide::Left, 20, "c");
    let (_, a) = register(&ws, &mut vcx, StatusSide::Left, 0, "a");
    let (_, b) = register(&ws, &mut vcx, StatusSide::Left, 10, "b");
    let (_, r2) = register(&ws, &mut vcx, StatusSide::Right, 5, "r2");
    let (_, r1) = register(&ws, &mut vcx, StatusSide::Right, -5, "r1");

    assert_eq!(order(&ws, &mut vcx, StatusSide::Left), [a, b, c]);
    assert_eq!(order(&ws, &mut vcx, StatusSide::Right), [r1, r2]);

    // The order is what is drawn: ascending priority runs left to right.
    assert!(left_edge(&mut vcx, "a") < left_edge(&mut vcx, "b"));
    assert!(left_edge(&mut vcx, "b") < left_edge(&mut vcx, "c"));
    assert!(left_edge(&mut vcx, "r1") < left_edge(&mut vcx, "r2"));
}

#[gpui::test]
fn left_items_sit_left_of_right_items(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    register(&ws, &mut vcx, StatusSide::Right, 0, "right");
    register(&ws, &mut vcx, StatusSide::Left, 0, "left");
    let window = bounds(&mut vcx, "status-bar").expect("the bar is drawn");
    let left = left_edge(&mut vcx, "left");
    let right = left_edge(&mut vcx, "right");
    assert!(left < right);
    let right_bounds = bounds(&mut vcx, "status-right").expect("right item");
    // The right group is pushed against the right edge of the bar.
    let gap = window.right() - right_bounds.right();
    assert!(
        f32::from(gap) < 20.,
        "right item hugs the right edge, gap {gap:?}"
    );
    assert!(left_edge(&mut vcx, "left") - f32::from(window.origin.x) < 20.);
}

#[gpui::test]
fn equal_priorities_keep_registration_order(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let (_, first) = register(&ws, &mut vcx, StatusSide::Left, 1, "first");
    let (_, second) = register(&ws, &mut vcx, StatusSide::Left, 1, "second");
    let (_, third) = register(&ws, &mut vcx, StatusSide::Left, 1, "third");
    let (_, early) = register(&ws, &mut vcx, StatusSide::Left, 0, "early");
    assert_eq!(
        order(&ws, &mut vcx, StatusSide::Left),
        [early, first, second, third]
    );
}

#[gpui::test]
fn hidden_item_keeps_its_slot_but_takes_no_room(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let (badge, id) = register(&ws, &mut vcx, StatusSide::Left, 0, "badge");
    register(&ws, &mut vcx, StatusSide::Left, 1, "after");
    let before = left_edge(&mut vcx, "after");

    badge.update(&mut vcx, |item, cx| item.set_visible(false, cx));
    vcx.run_until_parked();
    assert!(bounds(&mut vcx, "status-badge").is_none());
    assert!(
        left_edge(&mut vcx, "after") < before,
        "the next item moves left"
    );
    assert!(order(&ws, &mut vcx, StatusSide::Left).contains(&id));

    badge.update(&mut vcx, |item, cx| item.set_visible(true, cx));
    vcx.run_until_parked();
    assert!(bounds(&mut vcx, "status-badge").is_some());
    assert_eq!(left_edge(&mut vcx, "after"), before);
}

#[gpui::test]
fn an_item_updating_itself_redraws_the_bar(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let (item, _) = register(&ws, &mut vcx, StatusSide::Right, 0, "old");
    item.update(&mut vcx, |item, cx| item.set_label("new", cx));
    vcx.run_until_parked();
    assert!(bounds(&mut vcx, "status-new").is_some());
    assert!(bounds(&mut vcx, "status-old").is_none());
}

#[gpui::test]
fn removing_and_re_registering(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let (item, id) = register(&ws, &mut vcx, StatusSide::Left, 0, "x");
    register(&ws, &mut vcx, StatusSide::Left, 1, "y");

    let bar = vcx.update(|_, cx| ws.read(cx).status_bar().clone());
    let removed = bar.update(&mut vcx, |bar, cx| bar.remove_status_item(id, cx));
    assert!(removed);
    assert!(!bar.update(&mut vcx, |bar, cx| bar.remove_status_item(id, cx)));
    vcx.run_until_parked();
    assert!(bounds(&mut vcx, "status-x").is_none());

    // The same entity registered again lands where the new registration says, once.
    let again = vcx.update(|_, cx| {
        ws.update(cx, |ws, cx| {
            ws.register_status_item(StatusSide::Right, 3, item.clone(), cx)
        })
    });
    assert_eq!(again, id);
    let moved = vcx.update(|_, cx| {
        ws.update(cx, |ws, cx| {
            ws.register_status_item(StatusSide::Left, 9, item.clone(), cx)
        })
    });
    assert_eq!(moved, id);
    assert!(order(&ws, &mut vcx, StatusSide::Right).is_empty());
    assert_eq!(order(&ws, &mut vcx, StatusSide::Left).len(), 2);
    let placement = vcx.update(|_, cx| bar.read(cx).placement(id));
    assert_eq!(placement, Some((StatusSide::Left, 9)));
}

#[gpui::test]
fn an_idle_status_bar_never_redraws(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    register(&ws, &mut vcx, StatusSide::Left, 0, "a");
    register(&ws, &mut vcx, StatusSide::Right, 0, "b");

    let notified = Rc::new(Cell::new(0usize));
    let bar = vcx.update(|_, cx| ws.read(cx).status_bar().clone());
    let counter = notified.clone();
    let _subscription = vcx.update(|_, cx| {
        cx.observe(&bar, move |_: Entity<StatusBar>, _| {
            counter.set(counter.get() + 1)
        })
    });
    vcx.executor().advance_clock(Duration::from_secs(30));
    vcx.run_until_parked();
    assert_eq!(
        notified.get(),
        0,
        "the bar notifies only when an item or the registry changes"
    );
}

#[gpui::test]
fn the_status_bar_is_below_the_docks_at_full_width(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "item");
    let bar = bounds(&mut vcx, "status-bar").expect("the bar is drawn even when empty");
    let item = bounds(&mut vcx, "item-item").expect("the item is drawn");
    assert!(
        item.bottom() <= bar.origin.y + gpui::px(0.5),
        "{item:?} vs {bar:?}"
    );
    assert_eq!(item.origin.x, bar.origin.x);
    assert_eq!(item.size.width, bar.size.width);
    assert!(f32::from(bar.size.height) > 10.);
}
