//! [`NotifierPort`]: desktop notifications.
//!
//! # Adapter
//!
//! Implemented by `oxikube_notify_os` on the platform notification centre. Services in
//! `oxikube_app` decide *when* to notify (rollout finished, pod crash-looping); the
//! adapter only displays. Notifications carry no secrets: callers pass redacted text.

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;

/// How urgent a notification is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationLevel {
    /// Informational.
    Info,
    /// Succeeded.
    Success,
    /// Needs attention.
    Warning,
    /// Failed.
    Error,
}

/// A notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// Short headline.
    pub title: String,
    /// Detail line.
    pub body: String,
    /// Urgency.
    pub level: NotificationLevel,
    /// The cluster it concerns, if any (so a click can focus it).
    pub cluster: Option<ClusterId>,
    /// Replacement tag: a new notification with the same tag replaces the old one
    /// and [`NotifierPort::clear`] removes it.
    pub tag: Option<String>,
}

/// Shows and clears desktop notifications.
#[async_trait]
pub trait NotifierPort: Send + Sync {
    /// Shows `notification`. A notification the OS suppresses (permission denied,
    /// do-not-disturb) is not an error.
    async fn notify(&self, notification: &Notification) -> OxiResult<()>;

    /// Removes any shown notification with `tag`.
    async fn clear(&self, tag: &str) -> OxiResult<()>;
}
