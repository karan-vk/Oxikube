//! A click on an OS notification, in the main window (E14-S06).
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
use gpui::{App, Task, Window};
use oxikube_app::{ClickAction, OsNotificationService};
use oxikube_workspace::CommandDispatcher;

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
