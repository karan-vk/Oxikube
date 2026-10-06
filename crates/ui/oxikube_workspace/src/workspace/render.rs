//! The workspace view: the dock area, plus the handlers of the workspace actions and of every
//! panel's toggle action.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, prelude::FluentBuilder as _,
};

use super::Workspace;
use crate::{
    actions::{
        CloseActiveItem, ReopenClosedItem, SplitDown, SplitLeft, SplitRight, SplitUp,
        ToggleBottomDock, ToggleLeftDock, ToggleRightDock, ToggleZoom, WORKSPACE_KEY_CONTEXT,
    },
    pane::SplitDirection,
    panel::DockPosition,
};

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .id("workspace")
            .key_context(WORKSPACE_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .on_action(cx.listener(|this, _: &SplitLeft, window, cx| {
                this.split_active_pane(SplitDirection::Left, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SplitRight, window, cx| {
                this.split_active_pane(SplitDirection::Right, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SplitUp, window, cx| {
                this.split_active_pane(SplitDirection::Up, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SplitDown, window, cx| {
                this.split_active_pane(SplitDirection::Down, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseActiveItem, window, cx| {
                // Nothing to close here: let a workspace around this one (a cluster tab's
                // workspace inside the window's) close its own item, the tab.
                if !this.close_active_item(window, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &ReopenClosedItem, window, cx| {
                this.reopen_closed_item(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleLeftDock, window, cx| {
                this.toggle_dock(DockPosition::Left, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleRightDock, window, cx| {
                this.toggle_dock(DockPosition::Right, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleBottomDock, window, cx| {
                this.toggle_dock(DockPosition::Bottom, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleZoom, window, cx| {
                this.toggle_zoom(window, cx);
            }));

        // Each panel's own toggle action (its keymap entry and button) toggles that panel.
        for panel in &self.panels {
            let id = panel.handle.panel_id();
            let action = panel.handle.toggle_action(cx);
            root = root.on_boxed_action(
                action.as_ref(),
                cx.listener(move |this, _, window, cx| {
                    this.toggle_panel_by_id(id, window, cx);
                }),
            );
        }

        // An empty dock area still paints its split background; keep the plain window background
        // until there is something to lay out.
        let body = div()
            .id("workspace-body")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .when(!self.is_blank(), |this| this.child(self.dock_area.clone()));
        let row = div()
            .id("workspace-row")
            .flex()
            .flex_row()
            .flex_1()
            .min_h_0()
            .w_full()
            .when_some(self.strip.clone(), |this, strip| {
                this.child(div().flex_none().h_full().child(strip))
            })
            .child(body);

        // The status bar takes the bottom strip; the toast and modal layers are absolute, so
        // they overlay the whole workspace without taking part in its layout (modal on top).
        // A workspace embedded in another leaves all three to it.
        root.flex()
            .flex_col()
            .relative()
            .child(row)
            .when(!self.shared_layers, |this| {
                this.child(self.status_bar.clone())
                    .child(self.toast_layer.clone())
                    .child(self.modal_layer.clone())
            })
    }
}
