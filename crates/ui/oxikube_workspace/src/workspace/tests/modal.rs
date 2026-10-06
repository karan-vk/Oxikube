//! The modal layer: open, Escape, outside click, replacement, veto, focus return, tab trap, and
//! the confirmation dialog.

use std::{cell::Cell, rc::Rc};

use gpui::{
    AppContext as _, Entity, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, VisualTestContext,
    point, px,
};

use super::*;
use crate::{
    modal::{DialogModal, ModalLayer},
    test_support::TestModal,
};

fn layer(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Entity<ModalLayer> {
    vcx.update(|_, cx| ws.read(cx).modal_layer().clone())
}

fn open_modal(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    title: &'static str,
) -> Entity<TestModal> {
    let view = vcx.update(|window, cx| {
        let view = cx.new(|cx| TestModal::new(title, cx));
        ws.update(cx, |ws, cx| ws.show_modal(view.clone(), window, cx));
        view
    });
    vcx.run_until_parked();
    view
}

fn is_open(layer: &Entity<ModalLayer>, vcx: &mut VisualTestContext) -> bool {
    vcx.update(|_, cx| layer.read(cx).has_active_modal())
}

fn focused_in(vcx: &mut VisualTestContext, handle: &gpui::FocusHandle) -> bool {
    vcx.update(|window, cx| handle.contains_focused(window, cx))
}

#[gpui::test]
fn opening_a_modal_shows_it_and_focuses_it(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    assert!(!is_open(&layer, &mut vcx));
    assert!(
        bounds(&mut vcx, "modal-layer").is_none(),
        "no modal, no layer in the tree"
    );

    let modal = open_modal(&ws, &mut vcx, "a");
    assert!(is_open(&layer, &mut vcx));
    assert!(bounds(&mut vcx, "modal-a").is_some());
    let handle = vcx.update(|_, cx| gpui::Focusable::focus_handle(&modal, cx));
    assert!(focused_in(&mut vcx, &handle));
    // The scrim covers the whole window, the modal sits near the top, horizontally centred.
    let scrim = bounds(&mut vcx, "modal-layer").expect("scrim");
    let content = bounds(&mut vcx, "modal-a").expect("content");
    assert!(content.origin.y < scrim.origin.y + scrim.size.height / 3.);
    let centre_offset = (content.center().x - scrim.center().x).abs();
    assert!(centre_offset < px(1.), "{centre_offset:?}");
}

#[gpui::test]
fn escape_closes_the_modal_and_focus_returns_to_the_previous_element(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let item = open(&ws, &mut vcx, "editor");
    assert!(item_focused(&ws, &mut vcx, item));

    open_modal(&ws, &mut vcx, "a");
    assert!(!item_focused(&ws, &mut vcx, item), "the modal took focus");

    vcx.simulate_keystrokes("escape");
    assert!(!is_open(&layer, &mut vcx));
    assert!(bounds(&mut vcx, "modal-a").is_none());
    assert!(
        item_focused(&ws, &mut vcx, item),
        "focus is back on the editor"
    );

    // And nothing is left over: a later Escape does not reopen or close anything.
    vcx.simulate_keystrokes("escape");
    assert!(item_focused(&ws, &mut vcx, item));
}

#[gpui::test]
fn a_second_modal_replaces_the_first_and_focus_still_goes_home(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let item = open(&ws, &mut vcx, "editor");

    open_modal(&ws, &mut vcx, "a");
    open_modal(&ws, &mut vcx, "b");
    assert!(bounds(&mut vcx, "modal-a").is_none(), "the first is gone");
    assert!(bounds(&mut vcx, "modal-b").is_some());
    assert!(vcx.update(|_, cx| {
        layer
            .read(cx)
            .active_modal::<TestModal>()
            .is_some_and(|m| m.read(cx).title == "b")
    }));

    vcx.simulate_keystrokes("escape");
    assert!(!is_open(&layer, &mut vcx));
    assert!(
        item_focused(&ws, &mut vcx, item),
        "focus goes back to the editor, not into the dead modal"
    );
}

#[gpui::test]
fn toggle_modal_opens_then_closes_the_same_type(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let toggle = |vcx: &mut VisualTestContext| {
        vcx.update(|window, cx| {
            ws.update(cx, |ws, cx| {
                ws.toggle_modal(window, cx, |_, cx| TestModal::new("t", cx))
            });
        });
        vcx.run_until_parked();
    };
    toggle(&mut vcx);
    assert!(is_open(&layer, &mut vcx));
    toggle(&mut vcx);
    assert!(!is_open(&layer, &mut vcx));
}

#[gpui::test]
fn clicking_outside_closes_but_clicking_inside_does_not(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let item = open(&ws, &mut vcx, "editor");
    open_modal(&ws, &mut vcx, "a");
    let content = bounds(&mut vcx, "modal-a").expect("content");
    vcx.simulate_click(center(content), Modifiers::none());
    assert!(is_open(&layer, &mut vcx), "a click inside stays");

    vcx.simulate_click(point(px(2.), px(2.)), Modifiers::none());
    assert!(!is_open(&layer, &mut vcx), "a click on the scrim closes");
    assert!(
        item_focused(&ws, &mut vcx, item),
        "focus is back on the editor, not on the scrim the click landed on"
    );
}

#[gpui::test]
fn the_scrim_keeps_clicks_away_from_the_workspace(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let item = open(&ws, &mut vcx, "editor");
    let editor = bounds(&mut vcx, "item-editor").expect("item");
    let modal = open_modal(&ws, &mut vcx, "a");
    // Keep the modal open so the only way the item could take focus is the click reaching it.
    modal.update(&mut vcx, |modal, _| modal.veto = true);
    vcx.simulate_click(
        point(editor.origin.x + px(5.), editor.origin.y + px(5.)),
        Modifiers::none(),
    );
    assert!(
        !item_focused(&ws, &mut vcx, item),
        "the click did not focus the item under the scrim"
    );
}

#[gpui::test]
fn a_view_can_veto_dismissal(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let modal = open_modal(&ws, &mut vcx, "a");
    let asked = modal.update(&mut vcx, |modal, _| {
        modal.veto = true;
        modal.asked.clone()
    });
    vcx.simulate_keystrokes("escape");
    assert!(is_open(&layer, &mut vcx));
    assert_eq!(asked.get(), 1);
    vcx.simulate_click(point(px(2.), px(2.)), Modifiers::none());
    assert!(is_open(&layer, &mut vcx));
    assert_eq!(asked.get(), 2);

    modal.update(&mut vcx, |modal, _| modal.veto = false);
    vcx.simulate_keystrokes("escape");
    assert!(!is_open(&layer, &mut vcx));
}

#[gpui::test]
fn dismiss_event_from_the_view_closes_it(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let item = open(&ws, &mut vcx, "editor");
    let modal = open_modal(&ws, &mut vcx, "a");
    modal.update(&mut vcx, |modal, cx| modal.dismiss(cx));
    vcx.run_until_parked();
    assert!(!is_open(&layer, &mut vcx));
    assert!(item_focused(&ws, &mut vcx, item));
}

#[gpui::test]
fn tab_cycles_inside_the_modal_only(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    // Tab stops behind the modal that an untrapped Tab would walk into.
    let editor = open(&ws, &mut vcx, "editor");
    open(&ws, &mut vcx, "logs");
    let modal = open_modal(&ws, &mut vcx, "a");
    let (inside, first, second) = vcx.update(|_, cx| {
        let view = modal.read(cx);
        (
            gpui::Focusable::focus_handle(view, cx),
            view.first.clone(),
            view.second.clone(),
        )
    });

    // Walk the whole cycle several times in both directions: focus never leaves the modal, and
    // both of its tab stops are visited.
    for (key, rounds) in [("tab", 6), ("shift-tab", 6)] {
        let mut visited = Vec::new();
        for _ in 0..rounds {
            vcx.simulate_keystrokes(key);
            assert!(focused_in(&mut vcx, &inside), "{key} left the modal");
            assert!(!item_focused(&ws, &mut vcx, editor));
            visited.push(
                vcx.update(|window, _| (first.is_focused(window), second.is_focused(window))),
            );
        }
        assert!(
            visited.contains(&(true, false)) && visited.contains(&(false, true)),
            "{key}: both stops are visited: {visited:?}"
        );
    }
}

#[gpui::test]
fn a_dialog_modal_confirms_on_enter_and_cancels_on_escape(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let item = open(&ws, &mut vcx, "editor");
    let confirmed = Rc::new(Cell::new(0));
    let cancelled = Rc::new(Cell::new(0));

    let open_dialog = |vcx: &mut VisualTestContext| {
        let (confirmed, cancelled) = (confirmed.clone(), cancelled.clone());
        vcx.update(|window, cx| {
            ws.update(cx, |ws, cx| {
                ws.toggle_modal(window, cx, move |_, cx| {
                    DialogModal::new("Delete pod?", cx)
                        .message("This cannot be undone.")
                        .confirm_label("Delete")
                        .destructive()
                        .on_confirm(move |_, _| confirmed.set(confirmed.get() + 1))
                        .on_cancel(move |_, _| cancelled.set(cancelled.get() + 1))
                });
            });
        });
        vcx.run_until_parked();
    };

    open_dialog(&mut vcx);
    assert!(
        bounds(&mut vcx, "dialog-modal").is_some(),
        "the dialog renders from oxikube_ui's dialog parts"
    );
    vcx.simulate_keystrokes("enter");
    assert_eq!((confirmed.get(), cancelled.get()), (1, 0));
    assert!(!is_open(&layer, &mut vcx));
    assert!(item_focused(&ws, &mut vcx, item));

    open_dialog(&mut vcx);
    vcx.simulate_keystrokes("escape");
    assert_eq!((confirmed.get(), cancelled.get()), (1, 1));
    assert!(!is_open(&layer, &mut vcx));
}

/// A full Enter press (down then up), the way a keyboard delivers it. A button clicks on key-up.
fn press_enter(vcx: &mut VisualTestContext) {
    let keystroke = Keystroke::parse("enter").unwrap();
    vcx.simulate_event(KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    vcx.simulate_event(KeyUpEvent { keystroke });
}

#[gpui::test]
fn enter_activates_the_focused_dialog_button(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let confirmed = Rc::new(Cell::new(0));
    let cancelled = Rc::new(Cell::new(0));

    let open_dialog = |vcx: &mut VisualTestContext| {
        let (confirmed, cancelled) = (confirmed.clone(), cancelled.clone());
        vcx.update(|window, cx| {
            ws.update(cx, |ws, cx| {
                ws.toggle_modal(window, cx, move |_, cx| {
                    DialogModal::new("Delete pod?", cx)
                        .destructive()
                        .on_confirm(move |_, _| confirmed.set(confirmed.get() + 1))
                        .on_cancel(move |_, _| cancelled.set(cancelled.get() + 1))
                });
            });
        });
        vcx.run_until_parked();
    };

    // Tab moves focus onto the first button (Cancel); Enter must press it, not confirm.
    open_dialog(&mut vcx);
    vcx.simulate_keystrokes("tab");
    press_enter(&mut vcx);
    assert_eq!(
        (confirmed.get(), cancelled.get()),
        (0, 1),
        "Enter on the focused Cancel button cancels"
    );
    assert!(!is_open(&layer, &mut vcx));

    // A second Tab reaches the confirm button; Enter presses it.
    open_dialog(&mut vcx);
    vcx.simulate_keystrokes("tab tab");
    press_enter(&mut vcx);
    assert_eq!((confirmed.get(), cancelled.get()), (1, 1));
    assert!(!is_open(&layer, &mut vcx));
}

#[gpui::test]
fn the_dialog_buttons_work_with_the_mouse(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let layer = layer(&ws, &mut vcx);
    let confirmed = Rc::new(Cell::new(0));
    let counter = confirmed.clone();
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.toggle_modal(window, cx, move |_, cx| {
                DialogModal::new("Sure?", cx).on_confirm(move |_, _| counter.set(counter.get() + 1))
            });
        });
    });
    vcx.run_until_parked();
    let ok = bounds(&mut vcx, "dialog-confirm").expect("the OK button is drawn");
    vcx.simulate_click(center(ok), Modifiers::none());
    assert_eq!(confirmed.get(), 1);
    assert!(!is_open(&layer, &mut vcx));
}
