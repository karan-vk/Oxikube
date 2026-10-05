//! Panels: dockable views that live at an edge of the window (cluster sidebar, logs, events,
//! the agent thread).
//!
//! The shape follows Zed's `workspace::dock::Panel` design (written from scratch, no Zed code
//! copied): a panel names itself for persistence, says which [`DockPosition`] it wants and how big
//! it starts, offers an icon and the action that toggles it, and is told when it becomes active
//! or zoomed. [`PanelHandle`] is its object-safe handle.

mod handle;
mod tab;

use gpui::{Action, App, Context, EventEmitter, Focusable, Pixels, Render, SharedString, Window};
use oxikube_ui::{IconName, dock::DockPlacement};

pub use handle::PanelHandle;
pub(crate) use tab::PanelTab;

/// The edge of the window a dock sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DockPosition {
    /// Left of the centre panes.
    Left,
    /// Below the centre panes.
    Bottom,
    /// Right of the centre panes.
    Right,
}

impl DockPosition {
    /// Every dock position, in the order the docks are listed.
    pub const ALL: [DockPosition; 3] = [
        DockPosition::Left,
        DockPosition::Bottom,
        DockPosition::Right,
    ];

    /// The dock area's name for this edge.
    pub fn placement(self) -> DockPlacement {
        match self {
            DockPosition::Left => DockPlacement::Left,
            DockPosition::Bottom => DockPlacement::Bottom,
            DockPosition::Right => DockPlacement::Right,
        }
    }

    /// The dock position for a dock area placement; `None` for the centre.
    pub fn from_placement(placement: DockPlacement) -> Option<Self> {
        match placement {
            DockPlacement::Left => Some(DockPosition::Left),
            DockPlacement::Bottom => Some(DockPosition::Bottom),
            DockPlacement::Right => Some(DockPosition::Right),
            DockPlacement::Center => None,
        }
    }
}

/// What a panel tells the workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelEvent {
    /// Show and focus the panel (opens its dock).
    Activate,
    /// Close the panel's dock.
    Close,
    /// Fill the window with the panel's dock group.
    ZoomIn,
    /// Give the window back.
    ZoomOut,
}

/// A dockable side panel.
#[allow(unused_variables)]
pub trait Panel: Focusable + EventEmitter<PanelEvent> + Render + Sized {
    /// Names the panel in persisted layouts. Once chosen, never change it.
    fn persistent_name() -> &'static str;

    /// The key the panel's settings and state are stored under (for example `"logs_panel"`).
    fn panel_key() -> &'static str;

    /// The dock the panel is added to.
    fn position(&self, window: &Window, cx: &App) -> DockPosition;

    /// The dock size when the panel opens it, in **unscaled** logical pixels (the workspace
    /// applies UI zoom, see `oxikube_ui::u`).
    fn default_size(&self, window: &Window, cx: &App) -> Pixels;

    /// The smallest the dock may be resized to while this panel shows, unscaled.
    fn min_size(&self, window: &Window, cx: &App) -> Option<Pixels> {
        None
    }

    /// The icon of the panel's toggle button and tab.
    fn icon(&self, window: &Window, cx: &App) -> Option<IconName>;

    /// The tooltip of the panel's toggle button.
    fn icon_tooltip(&self, window: &Window, cx: &App) -> Option<SharedString> {
        None
    }

    /// The title of the panel's tab.
    fn title(&self, cx: &App) -> SharedString {
        Self::persistent_name().into()
    }

    /// The action that toggles the panel (bound in the keymap, shown on its button). The
    /// workspace handles it: dispatching it calls [`crate::Workspace::toggle_panel`].
    fn toggle_action(&self) -> Box<dyn Action>;

    /// Ordering among the panels of a dock and their toggle buttons: lower comes first.
    fn activation_priority(&self) -> u32;

    /// Called when the panel becomes, or stops being, the displayed panel of its dock group.
    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {}

    /// Called when the panel's dock group zooms in (fills the window) or out. The workspace owns
    /// the zoom state; read it with [`crate::Workspace::is_zoomed`].
    fn set_zoomed(&mut self, zoomed: bool, window: &mut Window, cx: &mut Context<Self>) {}

    /// The panel's own state for layout persistence. Never put secrets here: it is written to
    /// disk with the layout.
    fn serialize(&self, cx: &App) -> Option<serde_json::Value> {
        None
    }

    /// Applies the state [`Panel::serialize`] produced in an earlier session, when the layout is
    /// restored (after the panel was added). Unknown or outdated state must be ignored, not
    /// fatal.
    fn restore(&mut self, state: &serde_json::Value, window: &mut Window, cx: &mut Context<Self>) {}
}
