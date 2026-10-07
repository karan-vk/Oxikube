//! Restoring a saved dock (E05-S05): its size, visibility, the side panels' state and displayed
//! tab, and the [dockable](crate::Item::can_dock) items saved in it (E09-S07: the terminals of
//! the bottom dock), rebuilt through the [`ItemRegistry`](crate::ItemRegistry) like centre items.

use gpui::{Context, Window};
use oxikube_ui::{
    UiScale, Unscaled,
    dock::{DockPlacement, DockState, PanelId, PanelInfo, PanelState, panel_handle},
};
use serde_json::Value;

use super::Workspace;
use crate::{item::ITEM_PANEL_NAME, panel::DockPosition, persistence::RestoreReport};

/// The largest share of the window a restored dock may take, so a layout saved on a big display
/// cannot bury the centre on a small one.
const MAX_DOCK_SHARE: f32 = 0.8;

/// One saved tab of a dock: its leaf, and whether its group displayed it.
struct SavedLeaf<'a> {
    leaf: &'a PanelState,
    displayed: bool,
}

impl Workspace {
    /// Applies one saved dock to the dock of the same position. Returns whether the dock exists
    /// (a panel was added to it).
    pub(super) fn restore_dock(
        &mut self,
        position: DockPosition,
        saved: &DockState,
        report: &mut RestoreReport,
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
        self.restore_dock_tabs(position, saved.panel(), report, window, cx);
        true
    }

    /// Hands every saved side panel its state, rebuilds the saved items (unless the dock holds
    /// items already: restore never duplicates what is open), and displays the saved tab of
    /// each group.
    fn restore_dock_tabs(
        &mut self,
        position: DockPosition,
        saved: &PanelState,
        report: &mut RestoreReport,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placement = position.placement();
        let mut leaves = Vec::new();
        collect_leaves(saved, true, &mut leaves);
        let restore_items = self.dock_items(position, cx).is_empty();
        let mut used: Vec<PanelId> = Vec::new();
        let mut displayed: Vec<PanelId> = Vec::new();
        for SavedLeaf {
            leaf,
            displayed: shown,
        } in leaves
        {
            let panel_id = if leaf.panel_name == ITEM_PANEL_NAME {
                if !restore_items {
                    continue;
                }
                let Some(tab) = self.restore_item(leaf, report, window, cx) else {
                    continue;
                };
                let panel_id = PanelId::from(tab.entity_id());
                self.dock_area.update(cx, |area, cx| {
                    area.add_panel_view(panel_handle(tab), placement, None, window, cx);
                });
                panel_id
            } else {
                // The first live panel of this name in this dock that no saved leaf claimed yet.
                let Some((panel_id, handle)) = self
                    .panels
                    .iter()
                    .filter(|p| {
                        p.handle.persistent_name() == leaf.panel_name
                            && !used.contains(&p.panel_id)
                            && self.placement_of(p.panel_id, cx) == Some(placement)
                    })
                    .map(|p| (p.panel_id, p.handle.boxed_clone()))
                    .next()
                else {
                    continue;
                };
                used.push(panel_id);
                if let Some(state) = saved_state(leaf) {
                    handle.restore_state(&state, window, cx);
                }
                panel_id
            };
            if shown {
                displayed.push(panel_id);
            }
        }
        // Rebuilt items are appended and displayed as they come: select the saved ones last.
        for panel_id in displayed {
            self.dock_area
                .update(cx, |area, cx| area.select_panel(panel_id, window, cx));
        }
    }
}

/// A side panel's saved state (its `PanelInfo` carries `{ "state": ... }`).
fn saved_state(leaf: &PanelState) -> Option<Value> {
    match &leaf.info {
        PanelInfo::Panel(info) => info.get("state").cloned(),
        _ => None,
    }
}

/// Every leaf under `state` in saved order, with whether its group displays it (`displayed` is
/// what the caller's group says about `state`; a bare leaf outside a tab group is displayed).
fn collect_leaves<'a>(state: &'a PanelState, displayed: bool, out: &mut Vec<SavedLeaf<'a>>) {
    match &state.info {
        PanelInfo::Stack { .. } => {
            for child in &state.children {
                collect_leaves(child, true, out);
            }
        }
        PanelInfo::Tabs { active_index } => {
            for (ix, leaf) in state.children.iter().enumerate() {
                collect_leaves(leaf, ix == *active_index, out);
            }
        }
        PanelInfo::Panel(_) => out.push(SavedLeaf {
            leaf: state,
            displayed,
        }),
    }
}
