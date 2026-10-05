//! Items: what a centre tab shows (a resource list, a YAML editor, a log stream...).
//!
//! The shape follows Zed's `workspace::item` design (written from scratch, no Zed code copied):
//! an [`Item`] is a focusable, event-emitting, renderable entity that describes its tab
//! ([`TabContent`]: title, icon, dirty) and may veto or react to being closed. [`ItemHandle`] is
//! its object-safe handle, so the workspace can hold items of every type side by side and a
//! caller can get its concrete entity back with `downcast` on `dyn ItemHandle`.
//!
//! Items know nothing about Kubernetes or about where they are docked: the workspace places them
//! in panes, and a feature crate only implements [`Item`].

mod handle;
mod registry;
mod tab;

use gpui::{App, Context, Entity, EventEmitter, Focusable, Render, SharedString, Window};
use oxikube_ui::IconName;

pub use handle::ItemHandle;
pub use registry::{ItemBuilder, ItemRegistry, register_item};
pub use tab::ITEM_PANEL_NAME;
pub(crate) use tab::{ItemTab, ItemTabEvent};

/// What a tab shows for an item: its title, an optional icon, and whether it has unsaved changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabContent {
    /// The tab label.
    pub title: SharedString,
    /// The icon drawn before the title.
    pub icon: Option<IconName>,
    /// The item holds unsaved changes (a dot is drawn after the title).
    pub dirty: bool,
}

impl TabContent {
    /// A clean tab with `title` and no icon.
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            icon: None,
            dirty: false,
        }
    }

    /// Sets the icon.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Sets the dirty flag.
    pub fn dirty(mut self, dirty: bool) -> Self {
        self.dirty = dirty;
        self
    }
}

/// What an item tells the workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemEvent {
    /// The tab content (title, icon, dirty) changed: redraw the tab.
    UpdateTab,
    /// The item asks to be closed (the workspace still honours [`Item::can_close`]).
    CloseItem,
}

/// The content of a centre tab.
///
/// Every method but [`Item::tab_content`] has a default, so a minimal item is a view plus a
/// title. Methods that take `&self` and an `&App` must be cheap: they run while the tab bar
/// renders.
#[allow(unused_variables)]
pub trait Item: Focusable + EventEmitter<ItemEvent> + Render + Sized {
    /// Title, icon and dirty flag of the tab.
    fn tab_content(&self, cx: &App) -> TabContent;

    /// A key identifying what this item shows (for example `"pod/default/web-0"`). Opening an
    /// item whose key is already open activates the open one instead of adding a duplicate (see
    /// [`crate::OpenOptions::reuse_existing`]). `None`: never deduplicated.
    fn item_key(&self, cx: &App) -> Option<SharedString> {
        None
    }

    /// Whether the item may be closed now. `false` keeps its tab open (and hides its close
    /// button); the item is expected to say why, for example with a toast.
    fn can_close(&self, cx: &App) -> bool {
        true
    }

    /// Called once when the item leaves the workspace (closed by the user, by a command, or
    /// replaced). Release anything the item holds here: watches, streams, sessions.
    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {}

    /// Called when the item becomes, or stops being, the displayed tab of its pane. Inactive
    /// items keep their state but are not rendered.
    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {}

    /// A copy of this item for the new pane of a split. `None` (the default) moves the item into
    /// the new pane instead, when its pane has another item to keep.
    fn clone_on_split(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<Self>> {
        None
    }

    /// The kind name this item serialises under, if it can be rebuilt from [`Item::serialize`]
    /// through the [`ItemRegistry`]. Items without one cannot be reopened after closing and are
    /// not restored with the layout.
    fn serialized_kind() -> Option<&'static str> {
        None
    }

    /// The state that rebuilds this item (with the builder registered for
    /// [`Item::serialized_kind`]). Never put secrets here: it is kept in memory for
    /// reopen-closed and written to disk with the layout.
    fn serialize(&self, cx: &App) -> Option<serde_json::Value> {
        None
    }
}
