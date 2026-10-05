//! Splitting panes, moving items between them, and the active pane.

use gpui::Axis;

use super::*;
use crate::{
    actions::{SplitDown, SplitRight},
    pane::{Member, SplitDirection},
};

fn split(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    direction: SplitDirection,
) -> Option<PaneId> {
    let pane = vcx
        .update(|window, cx| ws.update(cx, |ws, cx| ws.split_active_pane(direction, window, cx)));
    vcx.run_until_parked();
    pane
}

#[gpui::test]
fn split_right_then_down_copies_a_cloneable_item(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let a = open_with(
        &ws,
        &mut vcx,
        "a",
        OpenOptions::default(),
        TestItem::cloneable,
    );
    let first = active_pane(&ws, &mut vcx).id();

    let right = split(&ws, &mut vcx, SplitDirection::Right).expect("split right");
    assert_ne!(right, first);
    let panes = panes(&ws, &mut vcx);
    assert_eq!(panes.len(), 2);
    assert_eq!(panes[0].items(), [a], "the original stays on the left");
    assert_eq!(
        titles(&ws, &mut vcx, &panes[1]),
        ["a"],
        "a copy on the right"
    );
    assert_ne!(panes[1].items()[0], a);
    let new_item = panes[1].items()[0];
    assert_eq!(
        active_pane(&ws, &mut vcx).id(),
        right,
        "the new pane is active"
    );
    assert!(
        item_focused(&ws, &mut vcx, new_item),
        "and its item focused"
    );

    let below = split(&ws, &mut vcx, SplitDirection::Down).expect("split down");
    let group = vcx.update(|_, cx| ws.read(cx).pane_group(cx));
    let Some(Member::Axis(row)) = group.root() else {
        panic!("a horizontal split at the root, got {:?}", group.root());
    };
    assert_eq!(row.axis, Axis::Horizontal);
    let Member::Axis(column) = &row.members[1] else {
        panic!("the right side is now a vertical split");
    };
    assert_eq!(column.axis, Axis::Vertical);
    assert_eq!(group.panes().len(), 3);
    assert_eq!(group.panes()[2].id(), below);
    assert!(bounds(&mut vcx, "item-a").is_some());
}

#[gpui::test]
fn split_moves_an_item_that_cannot_be_copied(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let a = open(&ws, &mut vcx, "a");
    assert!(
        split(&ws, &mut vcx, SplitDirection::Right).is_none(),
        "a lone item that cannot be copied has nothing to split from"
    );
    assert_eq!(panes(&ws, &mut vcx).len(), 1);

    let b = open(&ws, &mut vcx, "b");
    let right = split(&ws, &mut vcx, SplitDirection::Right).expect("split");
    let panes = panes(&ws, &mut vcx);
    assert_eq!(panes.len(), 2);
    assert_eq!(panes[0].items(), [a]);
    assert_eq!(panes[1].items(), [b]);
    assert_eq!(panes[1].id(), right);
}

#[gpui::test]
fn split_actions_dispatch_from_the_keymap(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open_with(
        &ws,
        &mut vcx,
        "a",
        OpenOptions::default(),
        TestItem::cloneable,
    );
    let prefix = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    vcx.simulate_keystrokes(&format!("{prefix}-k right"));
    vcx.run_until_parked();
    assert_eq!(panes(&ws, &mut vcx).len(), 2, "split right by key binding");
    vcx.dispatch_action(SplitDown);
    vcx.run_until_parked();
    assert_eq!(panes(&ws, &mut vcx).len(), 3);
    vcx.dispatch_action(SplitRight);
    vcx.run_until_parked();
    assert_eq!(panes(&ws, &mut vcx).len(), 4);
}

#[gpui::test]
fn move_item_between_panes_and_the_emptied_pane_disappears(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let a = open(&ws, &mut vcx, "a");
    let b = open(&ws, &mut vcx, "b");
    let c = open(&ws, &mut vcx, "c");
    let left = active_pane(&ws, &mut vcx).id();
    let right = vcx
        .update(|window, cx| {
            ws.update(cx, |ws, cx| {
                ws.move_item_to_split(c, SplitDirection::Right, window, cx)
            })
        })
        .expect("c moves to a new pane");
    vcx.run_until_parked();
    assert_eq!(active_pane(&ws, &mut vcx).id(), right);

    // a moves to the front of the right pane, displayed and focused there.
    let moved = vcx
        .update(|window, cx| ws.update(cx, |ws, cx| ws.move_item(a, right, Some(0), window, cx)));
    vcx.run_until_parked();
    assert!(moved);
    let panes_now = panes(&ws, &mut vcx);
    assert_eq!(panes_now.len(), 2);
    assert_eq!(panes_now[0].items(), [b]);
    assert_eq!(panes_now[1].items(), [a, c]);
    assert_eq!(panes_now[1].active_item(), Some(a));
    assert!(item_focused(&ws, &mut vcx, a));
    assert_eq!(active_pane(&ws, &mut vcx).id(), right);

    // Moving the left pane's last item away removes the pane.
    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.move_item(b, right, None, window, cx)));
    vcx.run_until_parked();
    let panes_now = panes(&ws, &mut vcx);
    assert_eq!(panes_now.len(), 1);
    assert_eq!(panes_now[0].id(), right);
    assert_eq!(panes_now[0].items(), [a, c, b]);
    assert!(vcx.update(|_, cx| ws.read(cx).pane_group(cx).pane(left).is_none()));
}

#[gpui::test]
fn closing_the_active_pane_falls_back_to_another(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let a = open(&ws, &mut vcx, "a");
    let b = open(&ws, &mut vcx, "b");
    let right = vcx
        .update(|window, cx| {
            ws.update(cx, |ws, cx| {
                ws.move_item_to_split(b, SplitDirection::Right, window, cx)
            })
        })
        .expect("split");
    vcx.run_until_parked();
    assert_eq!(active_pane(&ws, &mut vcx).id(), right);
    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.close_item(b, window, cx)));
    vcx.run_until_parked();
    let pane = active_pane(&ws, &mut vcx);
    assert_eq!(pane.items(), [a]);
    assert!(
        item_focused(&ws, &mut vcx, a),
        "focus goes back to the remaining pane"
    );
}

#[gpui::test]
fn the_active_pane_follows_focus(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let a = open(&ws, &mut vcx, "a");
    let b = open(&ws, &mut vcx, "b");
    let right = vcx
        .update(|window, cx| {
            ws.update(cx, |ws, cx| {
                ws.move_item_to_split(b, SplitDirection::Right, window, cx)
            })
        })
        .expect("split");
    vcx.run_until_parked();
    let left = panes(&ws, &mut vcx)[0].id();
    assert_eq!(active_pane(&ws, &mut vcx).id(), right);

    // Focusing a's view (a click would do the same) makes its pane active.
    vcx.update(|window, cx| {
        let handle = ws.read(cx).item(a).expect("a").focus_handle(cx);
        handle.focus(window, cx);
        // Focus listeners run when the next frame is drawn.
        window.draw(cx).clear(cx);
    });
    vcx.run_until_parked();
    assert_eq!(active_pane(&ws, &mut vcx).id(), left);
    // New items go to the active pane.
    let c = open(&ws, &mut vcx, "c");
    assert_eq!(panes(&ws, &mut vcx)[0].items(), [a, c]);

    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.activate_pane(right, window, cx)));
    vcx.run_until_parked();
    assert_eq!(active_pane(&ws, &mut vcx).id(), right);
    assert!(item_focused(&ws, &mut vcx, b));
}
