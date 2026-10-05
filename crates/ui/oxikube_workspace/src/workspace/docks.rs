//! Dock snapshots and sizes, and zoom.

use gpui::{App, Context, EntityId, Pixels, Window, px};
use oxikube_ui::{UiScale, Unscaled, dock::PanelId, u};

use super::{Workspace, layout::first_group};
use crate::{dock::Dock, pane::PaneId, panel::DockPosition};

impl Workspace {
    /// A snapshot of the dock at `position`; `None` until a panel was added there.
    pub fn dock(&self, position: DockPosition, cx: &App) -> Option<Dock> {
        let area = self.dock_area.read(cx);
        let placement = position.placement();
        let tree = area.layout(placement)?;
        let size = area.dock_size(placement).unwrap_or(px(0.));
        Some(Dock {
            position,
            open: area.is_dock_open(placement),
            size: Unscaled::from_scaled(size, UiScale::get(cx)),
            panels: tree
                .panels()
                .filter_map(|panel| self.panel_entity(panel))
                .collect(),
            active_panel: self.active_dock_panel(position, cx),
        })
    }

    /// The entity id of the side panel carried by the dock panel `panel`.
    fn panel_entity(&self, panel: PanelId) -> Option<EntityId> {
        let docked = self.panels.iter().find(|p| p.panel_id == panel)?;
        Some(docked.handle.panel_id())
    }

    /// The side panel displayed by the first group of the dock at `position`.
    pub(super) fn active_dock_panel(&self, position: DockPosition, cx: &App) -> Option<EntityId> {
        let tree = self.dock_area.read(cx).layout(position.placement())?;
        let (_, displayed) = first_group(tree.root())?;
        self.panel_entity(displayed?)
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
    pub fn is_zoomed(&self, cx: &App) -> bool {
        self.dock_area.read(cx).is_zoomed()
    }

    /// The centre pane that is zoomed, if a centre pane is what is zoomed.
    pub fn zoomed_pane(&self, cx: &App) -> Option<PaneId> {
        let node = self.dock_area.read(cx).zoomed_group()?;
        self.pane_group(cx)
            .panes()
            .into_iter()
            .map(|pane| pane.id())
            .find(|pane| pane.node() == node)
    }

    fn min_dock_size(&self, position: DockPosition, window: &Window, cx: &App) -> Option<Pixels> {
        let active = self.active_dock_panel(position, cx)?;
        self.docked(active)?.handle.min_size(window, cx)
    }

    /// Keeps every dock at least as big as its displayed panel's [`Panel::min_size`] after a
    /// resize by drag. The dock area reports a divider drag only on release, so a dock dragged
    /// below the minimum follows the pointer and settles at the minimum when the button is let go.
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
