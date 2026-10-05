//! Docking: the dock area, its skin, the panel traits, and the layout tree the area keeps.
//!
//! The layout of every region (centre plus left, right and bottom docks) is a [`PaneTree`] of
//! splits and tab groups addressed by [`NodeId`]; panels are addressed by [`PanelId`]. The
//! workspace (`oxikube_workspace`) reads the tree and edits it through [`DockArea`]; nothing else
//! should need these lower-level names.

pub use gpui_component::Placement;
pub use gpui_component::dock::{
    BasePanel as PanelBehavior, BasePanelView as PanelBehaviorView, ClosePanel, DockArea,
    DockAreaState, DockEvent, DockLayout, DockPlacement, DockSkin, DragPanel, InsertTarget, NodeId,
    PaneNode, PaneRef, PaneTree, Panel, PanelControl, PanelEvent, PanelHandle, PanelId, PanelInfo,
    PanelState, PanelStyle, RootKind, TabGroup, TitleStyle, ToggleZoom, panel_handle,
    register_panel,
};
