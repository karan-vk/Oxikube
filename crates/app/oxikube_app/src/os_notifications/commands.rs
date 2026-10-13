//! `app::ToggleOsNotifications` and `app::SendTestNotification` on the command bus, each with its
//! MCP tool stub (`app.app_toggle_os_notifications`, `app.app_send_test_notification`).
//!
//! Neither changes a cluster: the first edits the user's own settings, the second shows one
//! harmless notification. Both therefore work on a read-only cluster and need no confirmation.

use std::sync::Arc;

use futures::future::BoxFuture;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::NotifierStatus;
use serde_json::json;

use super::service::{Delivery, OsNotificationService, unavailable_notice};
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// Writes `events.os_notifications` to the user's settings.
///
/// The app crate has no `gpui` and cannot see the settings store, so the binary implements this on
/// top of `oxikube_events_ui::EventsSettings::set_os_notifications` (comment-preserving, hot
/// reload). The future resolves once the value is in the store; the change then reaches the
/// service through [`OsNotificationService::apply`].
pub trait OsNotificationSettings: Send + Sync {
    /// Sets `events.os_notifications`.
    fn set_enabled(&self, enabled: bool) -> BoxFuture<'static, OxiResult<()>>;
}

/// Registers the two commands on `registry`. The binary installs it as
/// `oxikube_app::os_notifications`.
///
/// # Errors
///
/// A [`RegisterError`] when one of the ids is already taken.
pub fn register_commands(
    registry: &mut CommandRegistry,
    service: OsNotificationService,
    settings: Arc<dyn OsNotificationSettings>,
) -> Result<(), RegisterError> {
    let commands = Arc::new(Commands { service, settings });
    for id in [
        CommandId::APP_SEND_TEST_NOTIFICATION,
        CommandId::APP_TOGGLE_OS_NOTIFICATIONS,
    ] {
        let commands = commands.clone();
        let meta = *command::lookup(id).expect("declared in oxikube_domain::command");
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let commands = commands.clone();
            async move { commands.run(command).await }
        })?;
    }
    Ok(())
}

struct Commands {
    service: OsNotificationService,
    settings: Arc<dyn OsNotificationSettings>,
}

impl Commands {
    async fn run(&self, command: Command) -> OxiResult<CommandOutput> {
        match command {
            Command::AppToggleOsNotifications { enabled } => {
                let enabled = enabled.unwrap_or_else(|| !self.service.wanted());
                self.settings.set_enabled(enabled).await?;
                // The settings echo reaches the service through the binary's observer; applying
                // here makes the command correct on its own, and is a no-op when the echo came
                // first.
                let status = self.service.apply(enabled).await;
                Ok(CommandOutput {
                    message: Some(toggle_message(enabled, &status)),
                    data: Some(json!({
                        "enabled": enabled,
                        "status": status_id(&status),
                        "unavailable": unavailable_reason(&status),
                    })),
                })
            }
            Command::AppSendTestNotification => {
                let delivery = self.service.send_test().await;
                let (message, sent) = match &delivery {
                    Delivery::Sent => ("Test notification sent".to_owned(), true),
                    Delivery::Off => (
                        "OS notifications are off. Run \"Toggle OS Notifications\" to turn them on."
                            .to_owned(),
                        false,
                    ),
                    Delivery::Unavailable { hint } => (
                        hint.clone().unwrap_or_else(|| {
                            format!("This system cannot show OS notifications ({})", self.service.status())
                        }),
                        false,
                    ),
                    Delivery::Failed(why) => (format!("The OS refused the notification: {why}"), false),
                };
                Ok(CommandOutput {
                    message: Some(message),
                    data: Some(json!({
                        "sent": sent,
                        "status": status_id(&self.service.status()),
                    })),
                })
            }
            other => Err(OxiError::internal(format!(
                "{} is not an OS notification command",
                other.id()
            ))),
        }
    }
}

fn status_id(status: &NotifierStatus) -> &'static str {
    match status {
        NotifierStatus::Off => "off",
        NotifierStatus::Ready => "ready",
        NotifierStatus::Unavailable(_) => "unavailable",
    }
}

fn unavailable_reason(status: &NotifierStatus) -> Option<&str> {
    match status {
        NotifierStatus::Unavailable(reason) => Some(reason),
        NotifierStatus::Off | NotifierStatus::Ready => None,
    }
}

fn toggle_message(enabled: bool, status: &NotifierStatus) -> String {
    match (enabled, status) {
        (false, _) => "OS notifications are off".to_owned(),
        (true, NotifierStatus::Unavailable(reason)) => unavailable_notice(reason),
        (true, _) => {
            "OS notifications are on. \"Send Test OS Notification\" checks that they arrive."
                .to_owned()
        }
    }
}
