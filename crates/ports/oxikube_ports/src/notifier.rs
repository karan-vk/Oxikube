//! [`NotifierPort`]: desktop notifications.
//!
//! # Adapter
//!
//! Implemented by `oxikube_notify_os` on the platform notification centre (macOS
//! `NSUserNotificationCenter` / `UNUserNotificationCenter` through `notify-rust`, Linux
//! freedesktop notifications over D-Bus). Services in `oxikube_app` decide *when* to notify
//! (rollout finished, pod crash-looping, a Warning event) and whether the user opted in; the
//! adapter only displays. Notifications carry no secrets: callers pass redacted text, and
//! [`Notification::sanitised`] redacts and caps whatever is left.
//!
//! # Opt-in and clicks
//!
//! A notifier starts **off**. [`NotifierPort::set_enabled`] is the user turning OS
//! notifications on (the only moment an adapter may touch the notification service or ask the
//! OS for permission) or off again; [`NotifierPort::notify`] on a notifier that is off shows
//! nothing and succeeds. A click on a shown notification is reported to the handler given to
//! [`NotifierPort::on_activated`] as a [`NotificationActivation`]; the handler runs on an
//! adapter thread, so it only forwards (to a channel), and the app layer dispatches the
//! focus-window and navigate commands. Adapters never call into GPUI.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::{ErrorKind, OxiError, OxiResult};

/// The longest notification title, in characters. Longer titles end in an ellipsis.
pub const MAX_TITLE_CHARS: usize = 80;

/// The longest notification body, in characters. Longer bodies end in an ellipsis.
pub const MAX_BODY_CHARS: usize = 240;

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
    /// The object a click opens (the involved object of a Warning event), if any. The app layer
    /// turns it into a `resource::Open` command; it is never shown.
    pub action_target: Option<ResourceRef>,
}

impl Notification {
    /// A notification with no cluster, tag or click target.
    pub fn new(
        title: impl Into<String>,
        body: impl Into<String>,
        level: NotificationLevel,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            level,
            cluster: None,
            tag: None,
            action_target: None,
        }
    }

    /// The same notification, concerning `cluster`.
    #[must_use]
    pub fn with_cluster(mut self, cluster: ClusterId) -> Self {
        self.cluster = Some(cluster);
        self
    }

    /// The same notification, replacing (and clearable by) `tag`.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// The same notification, opening `target` (and focusing its cluster) when clicked.
    #[must_use]
    pub fn with_action_target(mut self, target: ResourceRef) -> Self {
        self.cluster.get_or_insert_with(|| target.cluster.clone());
        self.action_target = Some(target);
        self
    }

    /// The text that may leave the app: secrets scrubbed by `oxikube_domain::redact`, control
    /// characters (newlines included) turned into spaces, and the title and body capped at
    /// [`MAX_TITLE_CHARS`] and [`MAX_BODY_CHARS`]. Idempotent; the click target and tag are
    /// untouched because they never leave the process.
    #[must_use]
    pub fn sanitised(&self) -> Self {
        Self {
            title: clean_text(&self.title, MAX_TITLE_CHARS),
            body: clean_text(&self.body, MAX_BODY_CHARS),
            ..self.clone()
        }
    }

    /// What a click on this notification reports.
    pub fn activation(&self) -> NotificationActivation {
        NotificationActivation {
            tag: self.tag.clone(),
            cluster: self.cluster.clone(),
            action_target: self.action_target.clone(),
        }
    }
}

/// Redacts `text`, flattens control characters and caps it at `max` characters.
fn clean_text(text: &str, max: usize) -> String {
    let redacted = oxikube_domain::redact::redact(text);
    let flat: String = redacted
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut capped: String = flat.chars().take(max.saturating_sub(1)).collect();
    capped.truncate(capped.trim_end().len());
    capped.push('\u{2026}');
    capped
}

/// The user clicked a shown notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationActivation {
    /// The tag of the notification, if it had one.
    pub tag: Option<String>,
    /// The cluster it concerned: the click focuses this cluster's tab.
    pub cluster: Option<ClusterId>,
    /// The object to open, if the notification named one.
    pub action_target: Option<ResourceRef>,
}

/// Receives clicks on notifications. Called on an adapter thread: forward and return.
pub type ActivationHandler = Arc<dyn Fn(NotificationActivation) + Send + Sync>;

/// Whether a notifier can show notifications right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifierStatus {
    /// Switched off by the user (the default): nothing is shown and the OS was never asked.
    Off,
    /// On, and the platform service accepted the request.
    Ready,
    /// On, but there is no notification service to talk to (no D-Bus daemon, an unbundled
    /// macOS build the OS does not let notify). The reason is a short sentence for the user.
    Unavailable(String),
}

impl fmt::Display for NotifierStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NotifierStatus::Off => f.write_str("OS notifications are off"),
            NotifierStatus::Ready => f.write_str("OS notifications are on"),
            NotifierStatus::Unavailable(reason) => {
                write!(f, "OS notifications are unavailable: {reason}")
            }
        }
    }
}

/// The error a notifier returns when the platform has no notification service. Maps to
/// [`ErrorKind::Unsupported`]; [`is_unavailable`] recognises it.
pub fn unavailable(reason: impl Into<String>) -> OxiError {
    OxiError::unsupported(format!("no notification service: {}", reason.into()))
}

/// Whether `error` says the platform has no notification service (see [`unavailable`]).
pub fn is_unavailable(error: &OxiError) -> bool {
    error.kind() == ErrorKind::Unsupported
}

/// Shows and clears desktop notifications.
///
/// # Effects
///
/// Shows or clears desktop notifications; not a cluster mutation.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) when the platform has no
/// notification service ([`unavailable`]), [`Internal`](oxikube_domain::ErrorKind::Internal)
/// when the service fails. A notification the OS suppresses (permission denied,
/// do-not-disturb) or that arrives while the notifier is off is `Ok`.
#[async_trait]
pub trait NotifierPort: Send + Sync {
    /// Turns the notifier on or off. Starts off. Turning it on is the opt-in: the adapter
    /// connects to the platform service and asks for permission here and nowhere else, then
    /// reports whether notifications can be shown. Turning it off is always `Ok(Off)`.
    async fn set_enabled(&self, enabled: bool) -> OxiResult<NotifierStatus>;

    /// Shows `notification`. A notification the OS suppresses (permission denied,
    /// do-not-disturb) or that arrives while the notifier is off is not an error.
    async fn notify(&self, notification: &Notification) -> OxiResult<()>;

    /// Removes any shown notification with `tag`, where the platform allows it.
    async fn clear(&self, tag: &str) -> OxiResult<()>;

    /// Sets the handler told when the user clicks a shown notification, replacing the previous
    /// one. The handler runs on an adapter thread and must not block.
    fn on_activated(&self, handler: ActivationHandler);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitised_redacts_flattens_and_caps() {
        let n = Notification::new(
            "Pod\ncrashed",
            format!("token=abcdef1234567890abcdef\n{}", "x".repeat(400)),
            NotificationLevel::Warning,
        )
        .sanitised();
        assert_eq!(n.title, "Pod crashed");
        assert!(!n.body.contains("abcdef1234567890abcdef"), "{}", n.body);
        assert!(n.body.chars().count() <= MAX_BODY_CHARS);
        assert!(n.body.ends_with('\u{2026}'));
        assert_eq!(n.sanitised(), n, "idempotent");
    }

    #[test]
    fn short_text_is_untouched() {
        let n = Notification::new("Done", "Rollout finished", NotificationLevel::Success);
        assert_eq!(n.sanitised(), n);
    }

    #[test]
    fn unavailable_round_trips() {
        assert!(is_unavailable(&unavailable("no daemon")));
        assert!(!is_unavailable(&OxiError::internal("boom")));
        assert_eq!(
            NotifierStatus::Unavailable("no daemon".into()).to_string(),
            "OS notifications are unavailable: no daemon"
        );
    }

    #[test]
    fn action_target_names_the_cluster() {
        let target: ResourceRef = "3f2a9c01b7d4e865/apps/v1/Deployment/prod/api"
            .parse()
            .unwrap();
        let n = Notification::new("t", "b", NotificationLevel::Warning)
            .with_action_target(target.clone());
        assert_eq!(n.cluster.as_ref(), Some(&target.cluster));
        assert_eq!(n.activation().action_target, Some(target));
    }
}
