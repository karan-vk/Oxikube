//! Layout persistence on the workspace: capturing the layout as a [`SerializedWorkspace`] and
//! restoring one (E05-S05).
//!
//! Capture is the dock area's own dump (gpui-component `PanelState` trees; item tabs carry their
//! `{ kind, state }` descriptor) with dock sizes converted to unscaled pixels and unrestorable
//! items dropped. Restore rebuilds each saved item through the [`ItemRegistry`] and installs the
//! saved split and tab tree around them, so an unknown or declined item is skipped and the rest
//! still opens. Side panels belong to their feature crates and are added at startup, so restore
//! applies the saved size, visibility, displayed tab and panel state to the docks that exist.

use gpui::{App, Context, Window, px};
use oxikube_ui::{
    UiScale, Unscaled,
    dock::{DockLayout, DockPlacement, DockState, PanelId, PanelInfo, PanelState, panel_handle},
};
use serde_json::Value;

use super::Workspace;
use crate::{
    item::{ITEM_PANEL_NAME, ItemRegistry},
    panel::DockPosition,
    persistence::{
        LAYOUT_SCHEMA_VERSION, RestoreReport, SerializedWorkspace, SkipReason, SkippedItem,
        item_descriptor, prune, surviving_active,
    },
};

/// The largest share of the window a restored dock may take, so a layout saved on a big display
/// cannot bury the centre on a small one.
const MAX_DOCK_SHARE: f32 = 0.8;

impl Workspace {
    /// The layout as it is now, ready to store. [`SerializedWorkspace::window`] is left `None`:
    /// the workspace does not know its window's place on screen (the persistence controller fills
    /// it in).
    pub fn serialize_layout(&self, cx: &App) -> SerializedWorkspace {
        let scale = UiScale::get(cx);
        let mut state = self.dock_area.read(cx).dump(cx);
        for dock in [
            &mut state.left_dock,
            &mut state.bottom_dock,
            &mut state.right_dock,
        ] {
            if let Some(saved) = dock.take() {
                // Stored unscaled so a layout saved at 150 % zoom restores right at 100 %.
                let size = px(Unscaled::from_scaled(saved.size(), scale).0);
                *dock = Some(DockState::new(
                    saved.panel().clone(),
                    saved.placement(),
                    size,
                    saved.open(),
                ));
            }
        }
        prune(&mut state.center, &mut |leaf| {
            item_descriptor(leaf).is_some()
        });

        // The active pane counts only panes that survive the prune above.
        let restorable = |item| {
            self.item(item)
                .is_some_and(|i| i.serialized_kind().is_some() && i.serialize(cx).is_some())
        };
        let active = self.active_pane(cx).map(|p| p.id());
        let mut index = 0;
        let mut active_pane = None;
        for pane in self.pane_group(cx).panes() {
            if !pane.items().iter().any(|id| restorable(*id)) {
                continue;
            }
            if Some(pane.id()) == active {
                active_pane = Some(index);
            }
            index += 1;
        }
        SerializedWorkspace {
            version: LAYOUT_SCHEMA_VERSION,
            window: None,
            active_pane,
            dock_area: state,
        }
    }

    /// Applies a saved layout.
    ///
    /// - Centre: when no item is open, every saved item is rebuilt through the
    ///   [`ItemRegistry`] and the saved splits and tab groups are installed around the ones that
    ///   rebuild. Items of an unknown kind, or whose builder declines, are skipped (and reported),
    ///   the rest open. When items are already open the centre is left alone.
    /// - Docks: for each saved dock that has a panel added already, the size (clamped to the
    ///   window and the panel's minimum), the open flag, the displayed tab of each group, and each
    ///   panel's saved state ([`Panel::restore`](crate::Panel::restore)).
    ///
    /// Call it after the side panels were added. Does not fire for what the user did before: it
    /// never closes an open item.
    pub fn restore_layout(
        &mut self,
        saved: &SerializedWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> RestoreReport {
        let mut report = RestoreReport::default();
        if self.items.is_empty() {
            // Per saved pane, in saved order: whether any of its items came back.
            let mut survived = Vec::new();
            if let Some(layout) = self.layout_from_state(
                &saved.dock_area.center,
                &mut report,
                &mut survived,
                window,
                cx,
            ) {
                self.dock_area
                    .update(cx, |area, cx| area.set_center(layout, window, cx));
                report.centre_restored = true;
                let group = self.pane_group(cx);
                let panes = group.panes();
                // The saved index counts the panes as saved; panes that lost every item are gone.
                let active = saved
                    .active_pane
                    .filter(|ix| survived.get(*ix).copied().unwrap_or(false))
                    .map(|ix| survived[..ix].iter().filter(|s| **s).count())
                    .and_then(|ix| panes.get(ix))
                    .or(panes.first())
                    .map(|pane| pane.id());
                self.active_pane = active;
                self.focus_active_item(window, cx);
            }
        } else {
            report.centre_kept = true;
        }
        for (position, dock) in [
            (DockPosition::Left, &saved.dock_area.left_dock),
            (DockPosition::Bottom, &saved.dock_area.bottom_dock),
            (DockPosition::Right, &saved.dock_area.right_dock),
        ] {
            if let Some(dock) = dock
                && self.restore_dock(position, dock, window, cx)
            {
                report.docks_restored.push(position);
            }
        }
        cx.notify();
        report
    }

    /// The layout described by `state`, with the items rebuilt and registered; `None` when no
    /// item of it can be rebuilt. `survived` gets one entry per saved pane, in order.
    fn layout_from_state(
        &mut self,
        state: &PanelState,
        report: &mut RestoreReport,
        survived: &mut Vec<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<DockLayout> {
        match &state.info {
            PanelInfo::Stack { sizes, .. } => {
                let mut layout = match state.info.axis() {
                    Some(gpui::Axis::Vertical) => DockLayout::v_split(),
                    _ => DockLayout::h_split(),
                };
                let mut any = false;
                for (ix, child) in state.children.iter().enumerate() {
                    if let Some(child) = self.layout_from_state(child, report, survived, window, cx)
                    {
                        // 0 is the dock area's "unconstrained" marker.
                        let size = sizes.get(ix).copied().filter(|s| *s > px(0.));
                        layout = layout.child(child, size);
                        any = true;
                    }
                }
                any.then_some(layout)
            }
            PanelInfo::Tabs { active_index } => {
                let mut layout = DockLayout::tabs();
                let mut survivors = Vec::new();
                for (ix, leaf) in state.children.iter().enumerate() {
                    if let Some(tab) = self.restore_item(leaf, report, window, cx) {
                        layout = layout.panel_view(panel_handle(tab), cx);
                        survivors.push(ix);
                    }
                }
                survived.push(!survivors.is_empty());
                (!survivors.is_empty())
                    .then(|| layout.active_index(surviving_active(*active_index, &survivors)))
            }
            // A bare item where a group belongs.
            PanelInfo::Panel(_) => {
                let tab = self.restore_item(state, report, window, cx);
                survived.push(tab.is_some());
                let tab = tab?;
                Some(DockLayout::tabs().panel_view(panel_handle(tab), cx))
            }
        }
    }

    /// Rebuilds one saved item tab and registers it; skips and reports it when it cannot.
    fn restore_item(
        &mut self,
        leaf: &PanelState,
        report: &mut RestoreReport,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Entity<crate::item::ItemTab>> {
        if leaf.panel_name != ITEM_PANEL_NAME {
            return None;
        }
        let Some(descriptor) = item_descriptor(leaf) else {
            report.skipped_items.push(SkippedItem {
                kind: String::new(),
                reason: SkipReason::NoDescriptor,
            });
            return None;
        };
        match ItemRegistry::build(&descriptor.kind, &descriptor.state, window, cx) {
            Some(item) => {
                report.restored_items += 1;
                Some(self.register_item(item, window, cx))
            }
            None => {
                let reason = if ItemRegistry::is_registered(cx, &descriptor.kind) {
                    SkipReason::Declined
                } else {
                    SkipReason::UnknownKind
                };
                tracing::warn!(kind = %descriptor.kind, ?reason, "saved item not restored");
                report.skipped_items.push(SkippedItem {
                    kind: descriptor.kind,
                    reason,
                });
                None
            }
        }
    }

    /// Applies one saved dock to the dock of the same position. Returns whether the dock exists
    /// (a panel was added to it).
    fn restore_dock(
        &mut self,
        position: DockPosition,
        saved: &DockState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let placement = position.placement();
        if !self.dock_area.read(cx).has_dock(placement) {
            return false;
        }
        let size = f32::from(saved.size());
        if size.is_finite() && size > 0. {
            let scale = UiScale::get(cx).factor();
            let viewport = window.viewport_size();
            let extent = match placement {
                DockPlacement::Bottom => viewport.height,
                _ => viewport.width,
            };
            let max = f32::from(extent) / scale * MAX_DOCK_SHARE;
            let clamped = if max >= 1. { size.min(max) } else { size };
            self.resize_dock(position, Unscaled(clamped), window, cx);
        }
        if self.dock_area.read(cx).is_dock_open(placement) != saved.open() {
            self.toggle_dock_area(placement, window, cx);
        }
        self.restore_dock_panels(placement, saved.panel(), window, cx);
        true
    }

    /// Selects each saved group's displayed panel and hands every panel its saved state.
    fn restore_dock_panels(
        &mut self,
        placement: DockPlacement,
        saved: &PanelState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut leaves = Vec::new();
        collect_panel_leaves(saved, &mut leaves);
        let mut used: Vec<PanelId> = Vec::new();
        for (name, state, displayed) in leaves {
            // The first live panel of this name in this dock that no saved leaf claimed yet.
            let Some((panel_id, handle)) = self
                .panels
                .iter()
                .filter(|p| {
                    p.handle.persistent_name() == name
                        && !used.contains(&p.panel_id)
                        && self.placement_of(p.panel_id, cx) == Some(placement)
                })
                .map(|p| (p.panel_id, p.handle.boxed_clone()))
                .next()
            else {
                continue;
            };
            used.push(panel_id);
            if let Some(state) = &state {
                handle.restore_state(state, window, cx);
            }
            if displayed {
                self.dock_area
                    .update(cx, |area, cx| area.select_panel(panel_id, window, cx));
            }
        }
    }

    /// A dock's size in unscaled pixels, for tests.
    #[cfg(test)]
    pub(crate) fn dock_size_unscaled(&self, position: DockPosition, cx: &App) -> Option<f32> {
        let size = self.dock_area.read(cx).dock_size(position.placement())?;
        Some(Unscaled::from_scaled(size, UiScale::get(cx)).0)
    }
}

/// Every side-panel leaf under `state` as `(name, saved panel state, displayed in its group)`.
fn collect_panel_leaves(state: &PanelState, out: &mut Vec<(String, Option<Value>, bool)>) {
    match &state.info {
        PanelInfo::Stack { .. } => {
            for child in &state.children {
                collect_panel_leaves(child, out);
            }
        }
        PanelInfo::Tabs { active_index } => {
            for (ix, leaf) in state.children.iter().enumerate() {
                let saved = match &leaf.info {
                    PanelInfo::Panel(info) => info.get("state").cloned(),
                    _ => None,
                };
                out.push((leaf.panel_name.clone(), saved, ix == *active_index));
            }
        }
        PanelInfo::Panel(info) => {
            out.push((state.panel_name.clone(), info.get("state").cloned(), true))
        }
    }
}
