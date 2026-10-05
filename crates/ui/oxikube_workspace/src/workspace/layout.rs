//! Queries on the dock area's layout trees (which region and group holds a dock panel, what a
//! group displays).

use gpui::App;
use oxikube_ui::dock::{DockPlacement, NodeId, PaneNode, PaneRef, PanelId};

use super::Workspace;

/// Every region of the dock area, centre first.
const REGIONS: [DockPlacement; 4] = [
    DockPlacement::Center,
    DockPlacement::Left,
    DockPlacement::Bottom,
    DockPlacement::Right,
];

impl Workspace {
    /// The region (centre or a dock) holding a dock panel. Tabs can be dragged between regions,
    /// so a side panel is not necessarily in the dock it was added to.
    pub(super) fn placement_of(&self, panel: PanelId, cx: &App) -> Option<DockPlacement> {
        let area = self.dock_area.read(cx);
        REGIONS.into_iter().find(|placement| {
            area.layout(*placement)
                .is_some_and(|tree| tree.find_panel_node(panel).is_some())
        })
    }

    /// The tab group holding a dock panel.
    pub(super) fn group_of(&self, panel: PanelId, cx: &App) -> Option<NodeId> {
        let placement = self.placement_of(panel, cx)?;
        self.dock_area
            .read(cx)
            .layout(placement)?
            .find_panel_node(panel)
    }

    /// Whether the dock panel `panel` (an item tab or a side panel tab) is the one its group
    /// displays.
    pub(super) fn is_displayed(&self, panel: PanelId, cx: &App) -> bool {
        let Some(placement) = self.placement_of(panel, cx) else {
            return false;
        };
        let area = self.dock_area.read(cx);
        let Some(tree) = area.layout(placement) else {
            return false;
        };
        let group = tree
            .find_panel_node(panel)
            .and_then(|node| tree.find_node(node));
        matches!(group.map(PaneNode::kind),
            Some(PaneRef::Tabs { panels, active_ix }) if panels.get(active_ix) == Some(&panel))
    }
}

/// The first tab group under `node` in pre-order (where `DockArea::add_panel_view` puts a new
/// panel), and the panel it displays.
pub(super) fn first_group(node: &PaneNode) -> Option<(NodeId, Option<PanelId>)> {
    match node.kind() {
        PaneRef::Tabs { panels, active_ix } => Some((node.id(), panels.get(active_ix).copied())),
        PaneRef::Split { children, .. } => children.iter().find_map(first_group),
    }
}
