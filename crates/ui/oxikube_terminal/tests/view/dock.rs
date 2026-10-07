//! One terminal, moved around: between two panes and into the bottom dock (by API and by dragging
//! its tab), it keeps its backend and its screen; it is never started again.

use gpui::{Entity, Modifiers, MouseButton, point, px};
use oxikube_terminal::view::{
    BackendDescriptor, TerminalPanel, TerminalView, ensure_terminal_panel,
};
use oxikube_testkit::fakes::TerminalCall;
use oxikube_workspace::test_support::TestItem;
use oxikube_workspace::{DockPosition, SplitDirection};

use super::*;

fn panel(h: &mut Harness) -> Entity<TerminalPanel> {
    let ws = h.ws.clone();
    let dispatcher: Rc<dyn CommandDispatcher> = h.recorder.clone();
    let panel = h.vcx.update(|window, cx| {
        ensure_terminal_panel(&ws, Some(cluster()), Some(dispatcher), window, cx)
    });
    h.vcx.run_until_parked();
    panel
}

fn dock_of(h: &mut Harness, view: &Entity<TerminalView>) -> Option<DockPosition> {
    let ws = h.ws.clone();
    h.vcx
        .update(|_, cx| ws.read(cx).item_dock(view.entity_id(), cx))
}

fn session_id(h: &mut Harness, view: &Entity<TerminalView>) -> gpui::EntityId {
    h.vcx
        .update(|_, cx| view.read(cx).terminal().expect("running").entity_id())
}

/// The calls that start or end a session: one `OutputStream` per backend that was started, a
/// `Kill` when it was ended.
fn lifecycle_calls(h: &Harness) -> Vec<TerminalCall> {
    h.backend(0)
        .calls()
        .into_iter()
        .filter(|call| matches!(call, TerminalCall::OutputStream | TerminalCall::Kill))
        .collect()
}

#[gpui::test]
fn the_terminal_panel_starts_the_bottom_dock_closed_and_is_added_once(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let first = panel(&mut h);
    let second = panel(&mut h);
    assert_eq!(first, second, "one panel per workspace");
    let ws = h.ws.clone();
    let bottom = h
        .vcx
        .update(|_, cx| ws.read(cx).dock(DockPosition::Bottom, cx))
        .expect("the bottom dock");
    assert!(!bottom.is_open(), "an empty dock takes no space");
    assert_eq!(bottom.panels(), [first.entity_id()]);

    // The panel's button asks for a terminal of its cluster.
    h.vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.toggle_dock(DockPosition::Bottom, window, cx)
        });
    });
    h.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let button = h
        .vcx
        .debug_bounds("terminal-panel-new")
        .expect("the New Terminal button");
    h.vcx.simulate_click(button.center(), Modifiers::none());
    assert_eq!(
        *h.recorder.0.borrow(),
        [Command::TerminalNew {
            cluster: Some(cluster())
        }]
    );
}

#[gpui::test]
fn moving_between_panes_and_into_the_dock_keeps_the_backend_and_the_screen(
    cx: &mut TestAppContext,
) {
    let mut h = harness(cx);
    panel(&mut h);
    let ws = h.ws.clone();
    h.vcx.update(|window, cx| {
        let item = TestItem::build("a", cx);
        ws.update(cx, |ws, cx| ws.open_item(item, window, cx));
    });
    let view = h.open(BackendDescriptor::local(None).with_shell("zsh", vec![]));
    h.backend(0).output("hello from the shell");
    h.frame();
    let session = session_id(&mut h, &view);

    // Into a pane of its own, to the right.
    let right = h.vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.move_item_to_split(view.entity_id(), SplitDirection::Right, window, cx)
        })
    });
    h.frame();
    assert!(right.is_some(), "a second pane");
    // Into the bottom dock.
    let docked = h.vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.move_item_to_dock(view.entity_id(), DockPosition::Bottom, window, cx)
        })
    });
    h.frame();
    assert!(docked);
    assert_eq!(dock_of(&mut h, &view), Some(DockPosition::Bottom));
    // And back to the first pane.
    let first = h.vcx.update(|_, cx| ws.read(cx).panes(cx)[0].id());
    h.vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.move_item(view.entity_id(), first, None, window, cx)
        })
    });
    h.frame();
    assert_eq!(dock_of(&mut h, &view), None);

    assert_eq!(h.launches().len(), 1, "never started again");
    assert_eq!(
        session_id(&mut h, &view),
        session,
        "the same session entity"
    );
    assert_eq!(
        lifecycle_calls(&h),
        [TerminalCall::OutputStream],
        "the same backend, still running"
    );
    assert_eq!(
        h.row(&view, 0),
        "hello from the shell",
        "the screen is kept"
    );
}

/// Drags the element tagged `from` onto the element tagged `to`.
fn drag(h: &mut Harness, from: &'static str, to: &'static str) {
    h.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let from = h.vcx.debug_bounds(from).expect("drag source").center();
    let to = h.vcx.debug_bounds(to).expect("drop target").center();
    h.vcx
        .simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    h.vcx.simulate_mouse_move(
        point(from.x + px(10.), from.y + px(4.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    h.vcx
        .simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    h.vcx.update(|window, cx| window.draw(cx).clear(cx));
    h.vcx
        .simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    h.vcx
        .simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    h.frame();
}

#[gpui::test]
fn dragging_the_tab_into_the_dock_and_out_again_keeps_the_terminal(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    panel(&mut h);
    let ws = h.ws.clone();
    h.vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.toggle_dock(DockPosition::Bottom, window, cx)
        });
        let item = TestItem::build("a", cx);
        ws.update(cx, |ws, cx| ws.open_item(item, window, cx));
    });
    let view = h.open(BackendDescriptor::local(None).with_shell("zsh", vec![]));
    h.backend(0).output("still here");
    h.frame();
    let session = session_id(&mut h, &view);

    drag(&mut h, "tab-zsh", "panel-tab-Terminal");
    assert_eq!(dock_of(&mut h, &view), Some(DockPosition::Bottom), "docked");

    drag(&mut h, "tab-zsh", "tab-a");
    assert_eq!(dock_of(&mut h, &view), None, "back in the pane");

    assert_eq!(h.launches().len(), 1);
    assert_eq!(session_id(&mut h, &view), session);
    assert_eq!(lifecycle_calls(&h), [TerminalCall::OutputStream]);
    assert_eq!(h.row(&view, 0), "still here");
}

#[gpui::test]
fn splitting_a_terminal_starts_a_fresh_one_beside_it(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let descriptor = BackendDescriptor::local(Some(cluster()))
        .with_shell("zsh", vec![])
        .in_dir("/work");
    let view = h.open(descriptor.clone());
    let ws = h.ws.clone();
    let pane = h.vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.split_active_pane(SplitDirection::Right, window, cx)
        })
    });
    h.frame();
    assert!(pane.is_some());
    assert_eq!(
        h.launches(),
        [descriptor.clone(), descriptor],
        "a second process, same descriptor"
    );
    let terminals = h
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<TerminalView>());
    assert_eq!(terminals.len(), 2);
    assert!(terminals.contains(&view), "the original stays");
}
