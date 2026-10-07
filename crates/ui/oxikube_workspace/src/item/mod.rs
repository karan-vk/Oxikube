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

use crate::cluster::ClusterMark;

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
    /// The cluster's colour and read-only lock, drawn before the title. `None` (or a plain
    /// mark) draws nothing. Set by cluster tabs; the item emits [`ItemEvent::UpdateTab`] when
    /// the mark changes, so the tab redraws on that change and not on every frame.
    pub cluster: Option<ClusterMark>,
}

impl TabContent {
    /// A clean tab with `title` and no icon.
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            icon: None,
            dirty: false,
            cluster: None,
        }
    }

    /// Sets the icon.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Sets the cluster mark (colour dot and read-only lock).
    pub fn cluster(mut self, mark: ClusterMark) -> Self {
        self.cluster = Some(mark);
        self
    }

    /// Sets the dirty flag.
    pub fn dirty(mut self, dirty: bool) -> Self {
        self.dirty = dirty;
        self
    }
}

/// How an item answers a request to close its tab ([`Item::close_requested`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseRequest {
    /// Close the tab now.
    Close,
    /// Keep the tab: the item is asking the user (or waiting for something) first, and closes
    /// itself afterwards by emitting [`ItemEvent::CloseItem`].
    Deferred,
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

    /// Whether a close the user asks for (the tab's close button, `workspace::CloseActiveItem`)
    /// goes through [`Item::close_requested`] first. The default `false` closes at once. An item
    /// that must ask before it goes (a cluster tab with operations running) returns `true`; its
    /// tab then draws its own close button, which asks the item instead of removing the tab.
    /// Closing it with [`Workspace::close_item`](crate::Workspace::close_item) or
    /// [`ItemEvent::CloseItem`] still closes at once.
    fn intercepts_close(&self, cx: &App) -> bool {
        false
    }

    /// The user asked to close this tab and [`Item::intercepts_close`] is `true`. Answer
    /// [`CloseRequest::Close`] to close now, or [`CloseRequest::Deferred`] after starting
    /// whatever asks (a dialog) and emit [`ItemEvent::CloseItem`] once the answer is yes. Runs
    /// inside the workspace's update: open dialogs from an event or a deferred call, not here.
    fn close_requested(&mut self, window: &mut Window, cx: &mut Context<Self>) -> CloseRequest {
        CloseRequest::Close
    }

    /// Called once when the item leaves the workspace (closed by the user, by a command, or
    /// replaced). Release anything the item holds here: watches, streams, sessions.
    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {}

    /// Called when the item becomes, or stops being, the displayed tab of its pane. Inactive
    /// items keep their state but are not rendered.
    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {}

    /// Whether the item may live in a dock as well as in the centre panes (Zed's terminal: a tab
    /// that moves between the panes and the bottom dock). The default `false` keeps the item in
    /// the centre: a tab of it dropped on a dock goes back to its pane. A dockable item opens in
    /// a dock with [`Workspace::open_item_in_dock`](crate::Workspace::open_item_in_dock), moves
    /// there with [`Workspace::move_item_to_dock`](crate::Workspace::move_item_to_dock) or a
    /// drag, and is saved and restored with the dock's layout.
    fn can_dock(&self, cx: &App) -> bool {
        false
    }

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
