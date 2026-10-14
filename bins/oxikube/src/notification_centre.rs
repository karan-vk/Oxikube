//! The notification centre in the running app (E14-S05): the glue between the app's
//! [`NotificationService`], the session follower, the state db and the OS notifier.
//!
//! [`start`] runs on each window mount and does its work once per app:
//!
//! * the muted clusters are read from the state db off the UI thread; until then no cluster is
//!   watched, so a muted cluster never toasts at launch;
//! * the session follower runs on the foreground executor (it only waits): an opened cluster is
//!   watched for Warning events through the `EventService` (the cluster's one feed), a closed
//!   one is forgotten;
//! * each watched cluster's pump runs on the background executor, so an app with no open
//!   cluster has no task on the Tokio runtime for notifications;
//! * the raised notifications go to the [`OsNotificationService`], which shows them on the
//!   desktop only once the user opted in (`events.os_notifications`).
//!
//! The window's side (panel, badge, toasts) is `mount::notifications`.

use std::sync::Arc;

use gpui::{App, AppContext as _, Global, Task};
use oxikube_app::store::Spawner;
use oxikube_app::{NotificationService, OsNotificationService};

/// What [`start`] keeps running for the life of the app.
struct CentreFollow {
    _load: Task<()>,
    _sessions: Task<()>,
    _os: Task<()>,
}

impl Global for CentreFollow {}

/// Starts the centre for the app (idempotent: a second window reuses what the first started).
pub fn start(service: &NotificationService, os: &OsNotificationService, cx: &mut App) {
    if cx.has_global::<CentreFollow>() {
        return;
    }
    let executor = cx.background_executor().clone();
    let spawner: Arc<dyn Spawner> = Arc::new(move |task| executor.spawn(task).detach());
    service.attach(spawner);
    let load = cx.background_spawn(service.load());
    let follow = service.follow();
    let sessions = cx.spawn(async move |_| follow.await);
    let forward = oxikube_app::notifications::forward_to_os(service.clone(), os.clone());
    let os = cx.spawn(async move |_| forward.await);
    cx.set_global(CentreFollow {
        _load: load,
        _sessions: sessions,
        _os: os,
    });
}
