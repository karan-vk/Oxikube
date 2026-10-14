//! The notification centre in the window (E14-S05): the panel, the status-bar badge and the
//! toasts over the app's `NotificationService` (`oxikube_app::notifications`).
//!
//! | File | Holds |
//! |---|---|
//! | `centre.rs` | [`NotificationCentre`]: the window's model of the service, rows derived once per change ([`NotificationRow`]), redraws coalesced to the frame; [`CentreEvent::Raised`] |
//! | `panel.rs`, `render.rs` | [`NotificationPanel`]: the right-dock panel, a virtualised list with unread marks, "Mark all read" and the shown cluster's mute toggle; key context `NotificationPanel` |
//! | `status.rs` | [`NotificationStatusItem`]: the bell and unread count; a click toggles the panel |
//! | `toasts.rs` | [`follow_toasts`]: raised notifications as toasts, at most [`MAX_TOASTS`] per [`TOAST_WINDOW`] plus one summary |
//! | `commands.rs` | `notifications::TogglePanel` on the bus ([`register_commands`], [`serve`]) |
//!
//! Every action is a bus command: `notifications::TogglePanel` (`cmd-shift-i` / `ctrl-shift-i`,
//! the status-bar bell, the palette), `notifications::MarkRead` (a row's click, `r`),
//! `notifications::MarkAllRead` (the header, `shift-r`), `notifications::MuteCluster` /
//! `UnmuteCluster` (the header's toggle, the palette), and opening a row dispatches
//! `cluster::Select` and `resource::Open` for its object. [`install`] mounts all of it in a
//! window's workspace.

mod centre;
mod commands;
mod panel;
mod render;
mod status;
mod toasts;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use gpui::{AppContext as _, Entity, Subscription, Window, actions};
use oxikube_app::ClusterSessionManager;
use oxikube_app::notifications::NotificationService;

pub use centre::{APP_LABEL, CentreEvent, NotificationCentre, NotificationRow};
pub use commands::{NotificationPanelSink, PanelRequest, register_commands, serve};
pub use panel::{NotificationPanel, PANEL_NAME};
pub use render::ROW_HEIGHT;
pub use status::NotificationStatusItem;
pub use toasts::{MAX_TOASTS, SUMMARY_KEY, TOAST_WINDOW, follow_toasts, toast_key};

use crate::cluster_tab::CommandDispatcher;
use crate::panel::DockPosition;
use crate::status_bar::StatusSide;
use crate::workspace::Workspace;

/// The key context of the panel.
pub const NOTIFICATION_PANEL_CONTEXT: &str = "NotificationPanel";

/// Where the badge sits in the status bar: on the right, after the feature badges.
pub const STATUS_PRIORITY: i32 = 200;

actions!(
    notifications,
    [
        /// Show and focus the notification centre, or hide it (`notifications::TogglePanel`).
        TogglePanel,
        /// Mark every notification read (`notifications::MarkAllRead`).
        MarkAllRead,
    ]
);

actions!(
    notification_panel,
    [
        /// Select the next row.
        SelectNext,
        /// Select the previous row.
        SelectPrevious,
        /// Open the selected row: mark it read, select its cluster, open its object.
        OpenSelected,
        /// Mark the selected row read (`notifications::MarkRead`).
        MarkSelectedRead,
        /// Close the panel.
        Close,
    ]
);

/// What a window's notification centre needs.
pub struct NotificationDeps {
    /// The app's centre.
    pub service: NotificationService,
    /// The sessions, to name the clusters.
    pub sessions: ClusterSessionManager,
    /// The window's dispatcher (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
}

/// What [`install`] mounted; keep it as long as the window lives.
pub struct NotificationWiring {
    /// The model.
    pub centre: Entity<NotificationCentre>,
    /// The panel in the right dock.
    pub panel: Entity<NotificationPanel>,
    /// The status-bar badge.
    pub status: Entity<NotificationStatusItem>,
    _toasts: Subscription,
}

/// Mounts the notification centre in `workspace`: the panel in the right dock (closed until
/// toggled), the badge on the right of the status bar, and the toasts.
pub fn install(
    workspace: &Entity<Workspace>,
    deps: NotificationDeps,
    window: &mut Window,
    cx: &mut gpui::App,
) -> NotificationWiring {
    let centre = cx.new(|cx| NotificationCentre::new(deps.service, deps.sessions, cx));
    let panel = cx.new(|cx| NotificationPanel::new(centre.clone(), deps.dispatcher.clone(), cx));
    let status = cx.new(|cx| NotificationStatusItem::new(&centre, deps.dispatcher.clone(), cx));
    workspace.update(cx, |ws, cx| {
        let had_dock = ws.dock(DockPosition::Right, cx).is_some();
        ws.add_panel(panel.clone(), window, cx);
        if !had_dock
            && ws
                .dock(DockPosition::Right, cx)
                .is_some_and(|d| d.is_open())
        {
            ws.toggle_dock(DockPosition::Right, window, cx);
        }
        ws.register_status_item(StatusSide::Right, STATUS_PRIORITY, status.clone(), cx);
    });
    let toasts = follow_toasts(&centre, workspace.downgrade(), deps.dispatcher, cx);
    NotificationWiring {
        centre,
        panel,
        status,
        _toasts: toasts,
    }
}
