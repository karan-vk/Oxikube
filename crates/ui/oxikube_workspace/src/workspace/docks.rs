//! Dock snapshots and sizes, and zoom.

use gpui::{Context, Window, px};
use oxikube_ui::{UiScale, Unscaled, dock::PanelId, u};

use super::{Workspace, layout::first_group};
use crate::{dock::Dock, panel::DockPosition};

impl Workspace {
    /// A snapshot of the dock at `position`; `None` until a panel was added there.
    pub fn dock(&self, position: DockPosition, cx: &gpui::App) -> Option<Dock> {
        let area = self.dock_area.read(cx);
        let placement = position.placement();
        let tree = area.layout(placement)?;
        let entity_of = |panel: PanelId| {
            self.panels
                .iter()
                .find(|p| p.panel_id == panel)
                .map(|p| p.handle.panel_id())
        };
        let size = area.dock_size(placement).unwrap_or(px(0.));
        Some(Dock {
            position,
            open: area.is_dock_open(placement),
            size: Unscaled::from_scaled(size, UiScale::get(cx)),
            panels: tree.panels().filter_map(entity_of).collect(),
            active_panel: first_group(tree.root())
                .and_then(|(_, displayed)| displayed)
                .and_then(entity_of),
        })
    }

    /// Resizes the dock at `position` to `size` (unscaled), no smaller than its displayed panel's
    /// [`Panel::min_size`](crate::Panel::min_size).
    pub fn resize_dock(
        &mut self,
        position: DockPosition,
        size: Unscaled,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let min = self.min_dock_size(position, window, cx);
        let size = size.to_pixels().max(min.map(u).unwrap_or(px(0.)));
        self.dock_area.update(cx, |area, cx| {
            area.set_dock_size(position.placement(), size, window, cx)
        });
    }

    /// Zooms the active pane in (it fills the window, docks hidden), or out when something is
    /// zoomed. When a side panel has focus, its dock group is zoomed instead.
    pub fn toggle_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dock_area.read(cx).is_zoomed() {
            self.dock_area
                .update(cx, |area, cx| area.set_zoomed_out(window, cx));
            cx.notify();
            return;
        }
        let focused_panel = self
            .panels
            .iter()
            .find(|p| p.handle.focus_handle(cx).contains_focused(window, cx))
            .map(|p| p.panel_id);
        let node = match focused_panel {
            Some(panel) => self.group_of(panel, cx),
            None => self.active_pane(cx).map(|pane| pane.id().node()),
        };
        if let Some(node) = node {
            self.dock_area
                .update(cx, |area, cx| area.set_zoomed_in(node, window, cx));
        }
        cx.notify();
    }

    /// Whether a pane or dock group fills the window.
    pub fn is_zoomed(&self, cx: &gpui::App) -> bool {
        self.dock_area.read(cx).is_zoomed()
    }

    /// The centre pane that is zoomed, if a centre pane is what is zoomed.
    pub fn zoomed_pane(&self, cx: &gpui::App) -> Option<crate::pane::PaneId> {
        let node = self.dock_area.read(cx).zoomed_group()?;
        self.pane_group(cx)
            .panes()
            .into_iter()
            .map(|pane| pane.id())
            .find(|pane| pane.node() == node)
    }

    fn min_dock_size(
        &self,
        position: DockPosition,
        window: &Window,
        cx: &gpui::App,
    ) -> Option<gpui::Pixels> {
        let active = self.dock(position, cx)?.active_panel?;
        let entry = self.panels.iter().find(|p| p.handle.panel_id() == active)?;
        entry.handle.min_size(window, cx)
    }

    /// Keeps every dock at least as big as its displayed panel's [`Panel::min_size`] after a
    /// resize by drag.
    pub(super) fn enforce_min_dock_sizes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for position in DockPosition::ALL {
            let Some(min) = self.min_dock_size(position, window, cx).map(u) else {
                continue;
            };
            let placement = position.placement();
            let current = self.dock_area.read(cx).dock_size(placement);
            if current.is_some_and(|size| size < min) {
                self.dock_area.update(cx, |area, cx| {
                    area.set_dock_size(placement, min, window, cx)
                });
            }
        }
    }
}
