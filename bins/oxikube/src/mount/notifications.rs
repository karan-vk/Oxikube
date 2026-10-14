//! Notifications in the main window: the notification centre (E14-S05) and a click on an OS
//! notification (E14-S06).
//!
//! [`mount_centre`] puts the centre in the window's workspace (`oxikube_workspace::notifications`):
//! the panel in the right dock (closed until `notifications::TogglePanel`: `cmd-shift-i` /
//! `ctrl-shift-i`, the status-bar bell, the palette), the bell with the unread count on the right
//! of the status bar, and the capped toasts of the raised notifications. The panel's mute toggle
//! follows the shown cluster tab ([`CentreMount::set_active_cluster`]).
//!
//! The notifier adapter reports a click on one of its own threads; the app's
//! [`OsNotificationService`] turns it into a [`ClickAction`] and hands it to the sink installed
//! here, which only forwards it over a channel. The task started by [`serve_clicks`] receives it on
//! the UI thread, brings the application and the window to the front, then dispatches the
//! action's commands (`cluster::Select`, `resource::Open`) through the window's
//! [`CommandDispatcher`], exactly like a key or a button. Nothing here touches the OS.

use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{App, Entity, Task, Window};
use oxikube_app::{ClickAction, ClusterSessionManager, NotificationService, OsNotificationService};
use oxikube_domain::ids::ClusterId;
use oxikube_workspace::notifications::{NotificationDeps, NotificationWiring, PanelRequest};
use oxikube_workspace::{CommandDispatcher, Workspace};

/// The notification centre of one window; keep it as long as the window lives.
pub struct CentreMount {
    wiring: NotificationWiring,
    _serve: Task<()>,
}

impl CentreMount {
    /// Makes `cluster` the one the panel's mute toggle acts on (the shown cluster tab).
    pub fn set_active_cluster(&self, cluster: Option<ClusterId>, cx: &mut App) {
        self.wiring
            .panel
            .update(cx, |panel, cx| panel.set_active_cluster(cluster, cx));
    }
}

/// Mounts the notification centre in `workspace` and serves `notifications::TogglePanel` from
/// `requests` (the bus handler's queue).
pub fn mount_centre(
    workspace: &Entity<Workspace>,
    service: NotificationService,
    sessions: ClusterSessionManager,
    dispatcher: Rc<dyn CommandDispatcher>,
    requests: mpsc::UnboundedReceiver<PanelRequest>,
    window: &mut Window,
    cx: &mut App,
) -> CentreMount {
    let deps = NotificationDeps {
        service,
        sessions,
        dispatcher,
    };
    let wiring = oxikube_workspace::notifications::install(workspace, deps, window, cx);
    let serve =
        oxikube_workspace::notifications::serve(requests, workspace.downgrade(), window, cx);
    CentreMount {
        wiring,
        _serve: serve,
    }
}

/// Starts serving the clicks on `service`'s notifications for `window`. The returned task lives as
/// long as the window; when it is dropped the sink forwards into the void.
pub fn serve_clicks(
    service: &OsNotificationService,
    dispatcher: Rc<dyn CommandDispatcher>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let (clicks, mut inbox) = mpsc::unbounded::<ClickAction>();
    service.on_click(Arc::new(move |action| {
        // The window may be gone; there is nobody to tell.
        let _ = clicks.unbounded_send(action);
    }));
    window.spawn(cx, async move |cx| {
        while let Some(action) = inbox.next().await {
            let dispatcher = dispatcher.clone();
            let shown = cx.update(|window, cx| {
                cx.activate(true);
                window.activate_window();
                for command in action.commands() {
                    dispatcher.dispatch(command, cx);
                }
            });
            if shown.is_err() {
                break;
            }
        }
    })
}
