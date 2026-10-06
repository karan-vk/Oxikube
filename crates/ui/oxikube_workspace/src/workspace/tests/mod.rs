//! `#[gpui::test]`s of the workspace: items, panes and splits, docks and panels, zoom, and tab
//! drag and drop, all through a real window with the dock area's skin.

mod dock_sizes;
mod docks;
mod drag;
mod items;
mod modal;
mod panes;
mod persist;
mod status_bar;
mod toast;

use gpui::{
    AppContext as _, Bounds, Entity, EntityId, Pixels, TestAppContext, VisualTestContext, point,
};
use oxikube_ui::root::Root;

use super::*;
use crate::{
    pane::Pane,
    test_support::{TestItem, TestPanel},
};

/// A window whose root hosts a fresh workspace, with the workspace's actions and the test item
/// builder registered.
pub(super) fn workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, VisualTestContext) {
    crate::test_support::open_workspace(cx)
}

/// Opens a [`TestItem`] titled `title` (built by `configure`) with `options`.
pub(super) fn open_with(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    title: &str,
    options: OpenOptions,
    configure: impl FnOnce(TestItem) -> TestItem,
) -> EntityId {
    let title = title.to_owned();
    let id = vcx.update(|window, cx| {
        let item = cx.new(|cx| configure(TestItem::new(title, cx)));
        ws.update(cx, |ws, cx| {
            ws.open_item_with(Box::new(item), options, window, cx)
        })
    });
    vcx.run_until_parked();
    id
}

/// Opens a plain [`TestItem`] titled `title` in the active pane.
pub(super) fn open(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, title: &str) -> EntityId {
    open_with(ws, vcx, title, OpenOptions::default(), |item| item)
}

/// Adds a [`TestPanel`] titled `title` at `position`.
pub(super) fn add_panel(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    position: DockPosition,
    title: &str,
    configure: impl FnOnce(&mut TestPanel),
) -> Entity<TestPanel> {
    let panel = vcx.update(|window, cx| {
        let panel = TestPanel::build(position, title, cx);
        panel.update(cx, |panel, _| configure(panel));
        ws.update(cx, |ws, cx| ws.add_panel(panel.clone(), window, cx));
        panel
    });
    vcx.run_until_parked();
    panel
}

/// A snapshot of the dock at `position`, which must exist.
pub(super) fn dock(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    position: DockPosition,
) -> crate::Dock {
    vcx.update(|_, cx| ws.read(cx).dock(position, cx))
        .expect("the dock exists")
}

/// The centre panes now.
pub(super) fn panes(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Vec<Pane> {
    vcx.update(|_, cx| ws.read(cx).panes(cx))
}

/// The active pane now.
pub(super) fn active_pane(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Pane {
    vcx.update(|_, cx| ws.read(cx).active_pane(cx))
        .expect("an active pane")
}

/// The titles of `pane`'s items, in tab order.
pub(super) fn titles(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    pane: &Pane,
) -> Vec<String> {
    vcx.update(|_, cx| {
        let ws = ws.read(cx);
        pane.items()
            .iter()
            .map(|id| {
                ws.item(*id)
                    .expect("open item")
                    .tab_content(cx)
                    .title
                    .to_string()
            })
            .collect()
    })
}

/// Draws a frame and returns the bounds of the element tagged `selector`.
pub(super) fn bounds(
    vcx: &mut VisualTestContext,
    selector: &'static str,
) -> Option<Bounds<Pixels>> {
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.debug_bounds(selector)
}

/// [`bounds`] for a selector built at run time.
pub(super) fn bounds_named(
    vcx: &mut VisualTestContext,
    selector: String,
) -> Option<Bounds<Pixels>> {
    bounds(vcx, Box::leak(selector.into_boxed_str()))
}

/// Whether the item `id` (or anything inside it) has focus.
pub(super) fn item_focused(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    id: EntityId,
) -> bool {
    vcx.update(|window, cx| {
        ws.read(cx)
            .item(id)
            .is_some_and(|item| item.focus_handle(cx).contains_focused(window, cx))
    })
}

/// Whether `panel` has focus.
pub(super) fn panel_focused(vcx: &mut VisualTestContext, panel: &Entity<TestPanel>) -> bool {
    vcx.update(|window, cx| {
        use gpui::Focusable as _;
        panel.read(cx).focus_handle(cx).contains_focused(window, cx)
    })
}

/// The centre of `bounds`.
pub(super) fn center(bounds: Bounds<Pixels>) -> gpui::Point<Pixels> {
    point(
        bounds.origin.x + bounds.size.width / 2.,
        bounds.origin.y + bounds.size.height / 2.,
    )
}
