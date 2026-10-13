//! OS (desktop) notifications behind the opt-in (E14-S06).
//!
//! [`OsNotificationService`] is what the rest of the app calls to put a notification on the
//! desktop: the NotificationService (E14-S05) hands it the Warning events that pass its rules,
//! and `app::SendTestNotification` hands it a test one. It owns the policy the
//! [`NotifierPort`](oxikube_ports::NotifierPort) adapter does not:
//!
//! * **Opt-in.** Nothing reaches the port while the `events.os_notifications` setting is off, and
//!   the port is told to switch on only when the user turns the setting on ([`apply`]); the
//!   adapter contacts the OS then and not before, so start-up pays nothing for it.
//! * **Safe text.** Every notification is redacted, flattened and capped ([`oxikube_ports::Notification::sanitised`])
//!   before the port sees it: no secrets and no raw payloads leave the process.
//! * **No platform service is not an error.** A system without a notification daemon yields
//!   [`Delivery::Unavailable`] with a one-time hint text, never a panic or a retry loop.
//! * **Clicks.** A click on a shown notification arrives as a [`ClickAction`]: the cluster to
//!   focus and the object to open. The service never touches the UI; the binary forwards the
//!   action to the window, which comes to the front and runs the commands
//!   ([`ClickAction::commands`]).
//!
//! # Commands
//!
//! `app::ToggleOsNotifications` (the opt-in, written to `settings.json`) and
//! `app::SendTestNotification` ([`register_commands`]). Neither changes a cluster, so neither goes
//! through a guard tier.
//!
//! [`apply`]: OsNotificationService::apply

mod click;
mod commands;
mod service;

pub use click::{ClickAction, ClickSink};
pub use commands::{OsNotificationSettings, register_commands};
pub use service::{Delivery, OsNotificationService};

#[cfg(test)]
mod tests;
