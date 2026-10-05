//! Side panels: adding them, toggling a panel or a dock, and the events panels send.

use gpui::{AppContext as _, Context, Entity, EntityId, Window};
use oxikube_ui::{
    dock::{DockPlacement, InsertTarget, PanelId, panel_handle},
    u,
};

use super::{DockedPanel, Workspace, layout::first_group};
use crate::panel::{DockPosition, Panel, PanelEvent, PanelHandle, PanelTab};

impl Workspace {
    /// Adds `panel` to the dock it asks for ([`Panel::position`]), creating the dock at the
    /// panel's [`Panel::default_size`] when it does not exist yet. Panels of a dock are ordered by
    /// [`Panel::activation_priority`]; the tab a dock was displaying stays displayed. Adding a
    /// panel twice does nothing.
    pub fn add_panel<T: Panel>(
        &mut self,
        panel: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .panels
            .iter()
            .any(|p| p.handle.panel_id() == panel.entity_id())
        {
            return;
        }
        let handle: Box<dyn PanelHandle> = Box::new(panel);
        let position = handle.position(window, cx);
        let size = u(handle.default_size(window, cx));
        let priority = handle.activation_priority(cx);
        let tab = cx.new(|_| PanelTab::new(handle.boxed_clone()));
        let panel_id = PanelId::from(tab.entity_id());

        let workspace = cx.weak_entity();
        let entity_id = handle.panel_id();
        let subscription = handle.subscribe_to_panel_events(
            window,
            cx,
            Box::new(move |event, window, cx| {
                let event = *event;
                workspace
                    .update(cx, |this, cx| {
                        this.on_panel_event(entity_id, event, window, cx)
                    })
                    .ok();
            }),
        );

        // Index among the dock's panels by priority, ties after the panels already there.
        let index = self
            .panels
            .iter()
            .filter(|p| p.position == position && p.handle.activation_priority(cx) <= priority)
            .count();
        let placement = position.placement();
        self.dock_area.update(cx, |area, cx| {
            let displayed = area
                .layout(placement)
                .and_then(|tree| first_group(tree.root()));
            area.add_panel_view(panel_handle(tab), placement, Some(size), window, cx);
            if let Some((node, displayed)) = displayed {
                let target = InsertTarget::Tabs {
                    node,
                    ix: Some(index),
                    activate: false,
                };
                area.move_panel(panel_id, target, window, cx);
                if let Some(displayed) = displayed {
                    area.select_panel(displayed, window, cx);
                }
            }
        });
        self.panels.push(DockedPanel {
            handle,
            panel_id,
            position,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// The panel of type `T`, if one was added.
    pub fn panel<T: Panel>(&self) -> Option<Entity<T>> {
        self.panels.iter().find_map(|p| p.handle.downcast::<T>())
    }

    /// Every added panel, in the order their toggle buttons go: by dock, then by
    /// [`Panel::activation_priority`].
    pub fn panels(&self, cx: &gpui::App) -> Vec<Box<dyn PanelHandle>> {
        let mut panels: Vec<_> = self.panels.iter().collect();
        panels.sort_by_key(|p| {
            let dock = DockPosition::ALL.iter().position(|d| *d == p.position);
            (dock, p.handle.activation_priority(cx))
        });
        panels.into_iter().map(|p| p.handle.boxed_clone()).collect()
    }

    /// Toggles the panel of type `T` (Zed's toggle-focus behaviour):
    ///
    /// - hidden (dock closed or another tab displayed): open its dock, display and focus it;
    /// - shown but not focused: focus it;
    /// - shown and focused: close its dock and give focus back to the active pane.
    ///
    /// Returns whether the panel is shown afterwards; `false` too when no such panel was added.
    pub fn toggle_panel<T: Panel>(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.panel::<T>().map(|panel| panel.entity_id()) else {
            return false;
        };
        self.toggle_panel_by_id(id, window, cx)
    }

    /// [`Self::toggle_panel`] for the panel whose entity id is `id` (for panels of a type that
    /// has several instances).
    pub fn toggle_panel_by_id(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(entry) = self.docked(id) else {
            return false;
        };
        let (panel_id, focus) = (entry.panel_id, entry.handle.focus_handle(cx));
        let Some(placement) = self.placement_of(panel_id, cx) else {
            return false;
        };
        // A side panel dragged into the centre has no dock to close: it can only be focused.
        let in_dock = placement != DockPlacement::Center;
        let open = !in_dock || self.dock_area.read(cx).is_dock_open(placement);
        let shown = open && self.is_displayed(panel_id, cx);
        if shown && focus.contains_focused(window, cx) {
            if in_dock {
                self.toggle_dock_area(placement, window, cx);
            }
            self.focus_active_item(window, cx);
            cx.notify();
            return !in_dock;
        }
        self.show_panel(panel_id, placement, window, cx);
        focus.focus(window, cx);
        cx.notify();
        true
    }

    /// Opens or closes the dock at `position`. Opening focuses the panel it displays; closing
    /// gives focus back to the active pane when it was inside the dock. Returns whether the dock
    /// is open afterwards.
    pub fn toggle_dock(
        &mut self,
        position: DockPosition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let placement = position.placement();
        if !self.dock_area.read(cx).has_dock(placement) {
            return false;
        }
        let had_focus = self.dock_has_focus(placement, window, cx);
        self.toggle_dock_area(placement, window, cx);
        let open = self.dock_area.read(cx).is_dock_open(placement);
        if open {
            if let Some(panel) = self.active_dock_panel(position, cx) {
                self.focus_panel(panel, window, cx);
            }
        } else if had_focus {
            self.focus_active_item(window, cx);
        }
        cx.notify();
        open
    }

    fn on_panel_event(
        &mut self,
        id: EntityId,
        event: PanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.docked(id) else {
            return;
        };
        let (panel_id, focus) = (entry.panel_id, entry.handle.focus_handle(cx));
        let Some(placement) = self.placement_of(panel_id, cx) else {
            return;
        };
        match event {
            PanelEvent::Activate => {
                self.show_panel(panel_id, placement, window, cx);
                focus.focus(window, cx);
            }
            PanelEvent::Close => {
                if self.dock_area.read(cx).is_dock_open(placement) {
                    self.toggle_dock_area(placement, window, cx);
                    self.focus_active_item(window, cx);
                }
            }
            PanelEvent::ZoomIn => {
                if let Some(node) = self.group_of(panel_id, cx) {
                    self.dock_area
                        .update(cx, |area, cx| area.set_zoomed_in(node, window, cx));
                }
            }
            PanelEvent::ZoomOut => {
                self.dock_area
                    .update(cx, |area, cx| area.set_zoomed_out(window, cx));
            }
        }
        cx.notify();
    }

    /// Opens the panel's dock if needed and displays the panel.
    fn show_panel(
        &mut self,
        panel: PanelId,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dock_area.update(cx, |area, cx| {
            if area.has_dock(placement) && !area.is_dock_open(placement) {
                area.toggle_dock(placement, window, cx);
            }
            area.select_panel(panel, window, cx);
        });
    }

    fn focus_panel(&self, id: EntityId, window: &mut Window, cx: &mut gpui::App) {
        if let Some(entry) = self.docked(id) {
            entry.handle.focus_handle(cx).focus(window, cx);
        }
    }

    fn dock_has_focus(&self, placement: DockPlacement, window: &Window, cx: &gpui::App) -> bool {
        self.panels.iter().any(|p| {
            self.placement_of(p.panel_id, cx) == Some(placement)
                && p.handle.focus_handle(cx).contains_focused(window, cx)
        })
    }
}
