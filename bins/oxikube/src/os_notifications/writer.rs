//! [`SettingsOsNotificationWriter`]: `app::ToggleOsNotifications`' way into `settings.json`.
//!
//! The command handler in `oxikube_app` runs on whatever task dispatched the command and knows
//! nothing of GPUI; the settings store is a GPUI global that only the foreground thread may touch.
//! The writer is the hand-over: [`OsNotificationSettings::set_enabled`] queues a request on a
//! channel and awaits its reply, and one foreground task (owned by the writer, dropped with it)
//! applies each request with [`EventsSettings::set_os_notifications`], the comment-preserving
//! editor, in order.

use futures::channel::{mpsc, oneshot};
use futures::future::BoxFuture;
use futures::{FutureExt as _, StreamExt as _};
use gpui::{App, Task};
use oxikube_app::os_notifications::OsNotificationSettings;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_events_ui::EventsSettings;

struct Request {
    enabled: bool,
    reply: oneshot::Sender<OxiResult<()>>,
}

/// Writes `events.os_notifications` to the user's `settings.json` through the settings store.
///
/// Keep it alive for as long as commands may run (it owns the foreground task that applies the
/// edits); hand an `Arc` of it to `oxikube_app::os_notifications::register_commands`.
pub struct SettingsOsNotificationWriter {
    requests: mpsc::UnboundedSender<Request>,
    _worker: Task<()>,
}

impl SettingsOsNotificationWriter {
    /// A writer applying edits on `cx`'s foreground executor.
    pub fn new(cx: &mut App) -> Self {
        let (requests, mut inbox) = mpsc::unbounded::<Request>();
        let worker = cx.spawn(async move |cx| {
            while let Some(Request { enabled, reply }) = inbox.next().await {
                let edit = cx.update(|cx| EventsSettings::set_os_notifications(cx, enabled));
                // The caller may have gone away (its dispatch was dropped): the edit still ran.
                let _ = reply.send(edit.await);
            }
        });
        Self {
            requests,
            _worker: worker,
        }
    }
}

impl OsNotificationSettings for SettingsOsNotificationWriter {
    fn set_enabled(&self, enabled: bool) -> BoxFuture<'static, OxiResult<()>> {
        let (reply, answer) = oneshot::channel();
        let sent = self.requests.unbounded_send(Request { enabled, reply });
        async move {
            sent.map_err(|_| OxiError::internal("the settings writer has shut down"))?;
            answer
                .await
                .map_err(|_| OxiError::internal("the settings writer stopped before answering"))?
        }
        .boxed()
    }
}
