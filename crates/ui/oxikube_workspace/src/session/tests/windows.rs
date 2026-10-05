//! Several windows: independent workspaces, closing one leaves the other, shared globals.

use gpui::{AppContext as _, TestAppContext, WindowBounds};

use super::{open_window, rem_size, setup};
use crate::{
    session::{NewWindow, ZoomIn, main_windows, open_new_window, windows::CASCADE},
    test_support::{TestItem, register_test_item},
    window::MainView,
    workspace::Workspace,
};
use gpui::{Entity, WindowHandle};
use oxikube_ui::root::Root;

fn workspace_of(handle: WindowHandle<Root>, cx: &mut TestAppContext) -> Entity<Workspace> {
    cx.update(|cx| {
        let root = handle.read(cx).expect("the window is open");
        let main = root
            .view()
            .clone()
            .downcast::<MainView>()
            .expect("the main view");
        main.read(cx).workspace().clone()
    })
}

#[gpui::test]
fn two_windows_have_independent_layouts_and_closing_one_leaves_the_other(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    cx.update(register_test_item);
    let (first, mut first_cx) = open_window(cx);
    let second = cx.update(open_new_window).expect("a second window opens");
    let mut second_cx = gpui::VisualTestContext::from_window(second.into(), cx);
    second_cx.run_until_parked();
    assert_eq!(cx.update(|cx| main_windows(cx).len()), 2);

    let first_ws = workspace_of(first, cx);
    let second_ws = workspace_of(second, cx);
    assert_ne!(first_ws.entity_id(), second_ws.entity_id());

    // An item in the first window's workspace is not in the second's.
    first_cx.update(|window, cx| {
        let item = cx.new(|cx| TestItem::new("Pods", cx));
        first_ws.update(cx, |ws, cx| {
            ws.open_item(item, window, cx);
        });
    });
    first_cx.run_until_parked();
    let (first_panes, second_panes) = cx.update(|cx| {
        (
            first_ws.read(cx).panes(cx).len(),
            second_ws.read(cx).panes(cx).len(),
        )
    });
    assert_eq!(first_panes, 1);
    assert_eq!(second_panes, 0, "the other window's layout is untouched");

    // Closing one leaves the other, with its layout.
    second_cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
    let remaining = cx.update(|cx| main_windows(cx));
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].window_id(), first.window_id());
    assert_eq!(cx.update(|cx| first_ws.read(cx).panes(cx).len()), 1);
}

#[gpui::test]
fn the_new_window_action_opens_another_window_next_to_the_active_one(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (first, mut vcx) = open_window(cx);
    let first_bounds = vcx.update(|window, _| window.window_bounds());
    vcx.dispatch_action(NewWindow);
    vcx.run_until_parked();
    let windows = cx.update(|cx| main_windows(cx));
    assert_eq!(windows.len(), 2);

    let other = windows
        .into_iter()
        .find(|w| w.window_id() != first.window_id())
        .expect("a new window");
    let other_bounds = cx.update(|cx| {
        other
            .update(cx, |_, window, _| window.window_bounds())
            .expect("the new window")
    });
    let (WindowBounds::Windowed(a), WindowBounds::Windowed(b)) = (first_bounds, other_bounds)
    else {
        panic!("both windows are plain windows");
    };
    assert_eq!(b.origin, a.origin + gpui::point(CASCADE, CASCADE));
    assert_eq!(a.size, b.size);
}

#[gpui::test]
fn windows_share_app_globals_and_only_views_are_per_window(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (_first, mut first_cx) = open_window(cx);
    let second = cx.update(open_new_window).expect("a second window");
    let mut second_cx = gpui::VisualTestContext::from_window(second.into(), cx);
    second_cx.run_until_parked();

    let first_base = rem_size(&mut first_cx);
    let second_base = rem_size(&mut second_cx);
    assert_eq!(first_base, second_base);
    // Zooming from either window zooms both: the zoom is a global.
    second_cx.update(|window, _| window.activate_window());
    second_cx.dispatch_action(ZoomIn);
    assert!((rem_size(&mut first_cx) - first_base * 1.1).abs() < 1e-3);
    assert!((rem_size(&mut second_cx) - second_base * 1.1).abs() < 1e-3);
}
