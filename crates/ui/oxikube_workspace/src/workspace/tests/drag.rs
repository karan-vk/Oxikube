//! Dragging a tab between panes with the mouse, through the dock area's tab bars, and an item tab
//! dropped on a dock.

use gpui::{Modifiers, MouseButton, point, px};

use super::*;
use crate::pane::SplitDirection;

/// Presses on `from`, drags past the drag threshold, moves over `to` and releases there.
fn drag(vcx: &mut VisualTestContext, from: &'static str, to: &'static str) {
    let from = center(bounds(vcx, from).unwrap_or_else(|| panic!("{from} is drawn")));
    let to = center(bounds(vcx, to).unwrap_or_else(|| panic!("{to} is drawn")));
    vcx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(
        point(from.x + px(10.), from.y + px(4.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
}

#[gpui::test]
fn dragging_a_tab_onto_another_panes_tab_moves_the_item(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let a = open(&ws, &mut vcx, "a");
    let b = open(&ws, &mut vcx, "b");
    let c = open(&ws, &mut vcx, "c");
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.move_item_to_split(c, SplitDirection::Right, window, cx)
        })
    })
    .expect("split");
    vcx.run_until_parked();

    drag(&mut vcx, "tab-a", "tab-c");

    let panes = panes(&ws, &mut vcx);
    assert_eq!(panes.len(), 2, "{panes:?}");
    assert_eq!(panes[0].items(), [b], "a left the first pane");
    assert!(
        panes[1].items().contains(&a),
        "a joined the second pane: {:?}",
        panes[1]
    );
    assert_eq!(panes[1].active_item(), Some(a), "and is displayed there");
    assert!(
        vcx.update(|_, cx| ws.read(cx).item(a).is_some()),
        "a moved, it was not closed"
    );
    assert!(vcx.update(|_, cx| ws.read(cx).closed_items().is_empty()));
}

#[gpui::test]
fn dragging_a_pane_s_last_tab_away_removes_the_pane(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let a = open(&ws, &mut vcx, "a");
    let b = open(&ws, &mut vcx, "b");
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.move_item_to_split(b, SplitDirection::Right, window, cx)
        })
    })
    .expect("split");
    vcx.run_until_parked();
    assert_eq!(panes(&ws, &mut vcx).len(), 2);

    drag(&mut vcx, "tab-b", "tab-a");

    let panes = panes(&ws, &mut vcx);
    assert_eq!(panes.len(), 1);
    assert!(panes[0].items().contains(&a) && panes[0].items().contains(&b));
}

#[gpui::test]
fn an_item_tab_dropped_on_a_dock_returns_to_its_pane(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let a = open(&ws, &mut vcx, "a");
    let b = open(&ws, &mut vcx, "b");
    let panel = add_panel(&ws, &mut vcx, DockPosition::Left, "tree", |_| {});

    drag(&mut vcx, "tab-a", "panel-tab-tree");

    let panes = panes(&ws, &mut vcx);
    assert_eq!(panes.len(), 1, "{panes:?}");
    assert_eq!(panes[0].items(), [a, b], "a is back at its own index");
    assert_eq!(panes[0].active_item(), Some(a), "and displayed");
    assert_eq!(
        vcx.update(|_, cx| ws.read(cx).active_item(cx).map(|item| item.item_id())),
        Some(a),
        "close-active-item and split reach it again"
    );
    assert!(item_focused(&ws, &mut vcx, a));
    let left = vcx
        .update(|_, cx| ws.read(cx).dock(DockPosition::Left, cx))
        .expect("the left dock");
    assert_eq!(
        left.panels(),
        [panel.entity_id()],
        "the dock only holds its panel"
    );
    assert_eq!(left.active_panel(), Some(panel.entity_id()));
    assert!(bounds(&mut vcx, "item-a").is_some());
    assert!(bounds(&mut vcx, "panel-tree").is_some());
    assert!(vcx.update(|_, cx| ws.read(cx).closed_items().is_empty()));
}

#[gpui::test]
fn an_item_dropped_on_a_dock_from_a_pane_it_emptied_goes_to_the_active_pane(
    cx: &mut TestAppContext,
) {
    let (ws, mut vcx) = workspace(cx);
    let a = open(&ws, &mut vcx, "a");
    let b = open(&ws, &mut vcx, "b");
    add_panel(&ws, &mut vcx, DockPosition::Right, "agent", |_| {});
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.move_item_to_split(b, SplitDirection::Right, window, cx)
        })
    })
    .expect("split");
    vcx.run_until_parked();

    drag(&mut vcx, "tab-b", "panel-tab-agent");

    let panes = panes(&ws, &mut vcx);
    assert_eq!(panes.len(), 1, "b's emptied pane is gone: {panes:?}");
    assert_eq!(panes[0].items(), [a, b]);
    assert_eq!(panes[0].active_item(), Some(b));
    assert_eq!(active_pane(&ws, &mut vcx).id(), panes[0].id());
    assert!(item_focused(&ws, &mut vcx, b));
}
