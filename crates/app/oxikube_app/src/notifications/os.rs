//! The centre's OS output: the raised notifications handed to the [`OsNotificationService`],
//! which shows them on the desktop only while the user opted in (E14-S06). A subscriber like any
//! view, so the centre itself knows nothing of the OS.

use futures::StreamExt as _;

use super::service::NotificationService;
use crate::os_notifications::{Delivery, OsNotificationService};

/// Hands every raised notification of `centre` to `os` until the centre is dropped. Each
/// notification goes out redacted, capped and tagged with its dedupe key (a later occurrence
/// replaces it on the desktop); the "no notification service" hint goes to the log once.
/// It only waits between notifications, so a host runs it on its foreground executor.
pub async fn forward_to_os(centre: NotificationService, os: OsNotificationService) {
    let mut changes = centre.subscribe();
    drop(centre);
    while let Some(change) = changes.next().await {
        for raised in &change.raised {
            match os.notify(&raised.to_os()).await {
                Delivery::Unavailable { hint: Some(hint) } => tracing::warn!("{hint}"),
                Delivery::Failed(why) => tracing::debug!(%why, "an OS notification failed"),
                Delivery::Sent | Delivery::Off | Delivery::Unavailable { hint: None } => {}
            }
        }
    }
}
