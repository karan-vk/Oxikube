//! [`Payload`]: what an OS notification says, derived from a [`Notification`] without calling
//! the OS, so the mapping is unit-testable on any machine.

use oxikube_ports::{Notification, NotificationLevel};

/// How loudly the platform should present a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    /// Informational and success messages: no sound, may be collapsed by the platform.
    Low,
    /// Warnings and errors.
    Normal,
}

impl Urgency {
    /// The urgency of a [`NotificationLevel`].
    pub const fn of(level: NotificationLevel) -> Self {
        match level {
            NotificationLevel::Info | NotificationLevel::Success => Urgency::Low,
            NotificationLevel::Warning | NotificationLevel::Error => Urgency::Normal,
        }
    }
}

/// The text and presentation of one OS notification. Title and body are already redacted,
/// flattened to one line and capped (`Notification::sanitised`). The click target is not part
/// of it: it never leaves the process, the notifier keeps it and reports it on a click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    /// Headline.
    pub title: String,
    /// Detail line.
    pub body: String,
    /// Presentation urgency.
    pub urgency: Urgency,
    /// The id of the shown notification this one replaces (same tag), where the platform
    /// supports replacement.
    pub replaces: Option<u32>,
}

impl Payload {
    /// The payload for `notification`, sanitising its text, replacing `replaces` if given.
    pub fn of(notification: &Notification, replaces: Option<u32>) -> Self {
        let clean = notification.sanitised();
        Self {
            title: clean.title,
            body: clean.body,
            urgency: Urgency::of(clean.level),
            replaces,
        }
    }
}

#[cfg(test)]
mod tests {
    use oxikube_ports::{MAX_BODY_CHARS, MAX_TITLE_CHARS};

    use super::*;

    #[test]
    fn text_is_capped_and_redacted() {
        let n = Notification::new(
            "t".repeat(500),
            format!("password=hunter2hunter2hunter2 {}", "b".repeat(500)),
            NotificationLevel::Warning,
        );
        let p = Payload::of(&n, None);
        assert!(p.title.chars().count() <= MAX_TITLE_CHARS);
        assert!(p.body.chars().count() <= MAX_BODY_CHARS);
        assert!(!p.body.contains("hunter2"), "{}", p.body);
    }

    #[test]
    fn newlines_become_spaces() {
        let n = Notification::new("a\nb", "c\r\nd\te", NotificationLevel::Info);
        let p = Payload::of(&n, Some(7));
        assert_eq!((p.title.as_str(), p.body.as_str()), ("a b", "c d e"));
        assert_eq!(p.replaces, Some(7));
    }

    #[test]
    fn urgency_follows_the_level() {
        assert_eq!(Urgency::of(NotificationLevel::Success), Urgency::Low);
        assert_eq!(Urgency::of(NotificationLevel::Error), Urgency::Normal);
    }

    #[test]
    fn the_click_target_is_not_in_the_payload() {
        let target = "3f2a9c01b7d4e865/v1/Pod/default/web-0".parse().unwrap();
        let n = Notification::new("t", "b", NotificationLevel::Warning).with_action_target(target);
        let p = Payload::of(&n, None);
        assert!(!format!("{p:?}").contains("web-0"));
    }
}
