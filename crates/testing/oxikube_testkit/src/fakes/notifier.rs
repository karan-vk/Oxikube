//! [`FakeNotifierPort`]: the desktop notifier without a desktop.

use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_ports::{
    ActivationHandler, Notification, NotificationActivation, NotifierPort, NotifierStatus,
};
use parking_lot::Mutex;

use crate::script::{CallLog, Script};

/// Queued responses for each [`FakeNotifierPort`] method.
#[derive(Debug, Default)]
pub struct NotifierScripts {
    /// `set_enabled`.
    pub set_enabled: Script<NotifierStatus>,
    /// `notify`.
    pub notify: Script<()>,
    /// `clear`.
    pub clear: Script<()>,
}

/// One call made on a [`FakeNotifierPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifierCall {
    /// `set_enabled(enabled)`.
    SetEnabled(bool),
    /// `notify(notification)`.
    Notify(Notification),
    /// `clear(tag)`.
    Clear(String),
}

/// Fake `NotifierPort`. Behaves like an adapter: it starts **off**, `set_enabled(true)`
/// answers `Ready` (scriptable, for example `Unavailable`), and a notification passed while it is
/// off is recorded but never shown. Fallback for `notify` and `clear`: success.
///
/// * [`notifications`](Self::notifications): every notification passed to `notify`, in order,
///   including ones that were refused or arrived while the notifier was off.
/// * [`shown`](Self::shown): the ones an on notifier would have displayed (the scripted result
///   was `Ok` and the notifier was on); a `clear(tag)` removes the ones with that tag.
/// * [`click`](Self::click) and [`click_last`](Self::click_last): the user clicking a shown
///   notification, which calls the handler set with `on_activated`.
#[derive(Default)]
pub struct FakeNotifierPort {
    script: NotifierScripts,
    calls: CallLog<NotifierCall>,
    enabled: AtomicBool,
    shown: Mutex<Vec<Notification>>,
    handler: Mutex<Option<ActivationHandler>>,
}

impl std::fmt::Debug for FakeNotifierPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeNotifierPort")
            .field("enabled", &self.is_enabled())
            .field("shown", &self.shown.lock().len())
            .finish_non_exhaustive()
    }
}

fake_plumbing!(FakeNotifierPort, NotifierScripts, NotifierCall);

impl FakeNotifierPort {
    /// A fake that is off, and accepts every notification once turned on.
    pub fn new() -> Self {
        Self::default()
    }

    /// A fake that is already on.
    pub fn enabled() -> Self {
        let fake = Self::default();
        fake.enabled.store(true, Ordering::SeqCst);
        fake
    }

    /// Whether the fake is on.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Every notification passed to `notify`, in order (including ones whose scripted
    /// response was an error and ones that arrived while the notifier was off).
    pub fn notifications(&self) -> Vec<Notification> {
        self.calls
            .calls()
            .into_iter()
            .filter_map(|c| match c {
                NotifierCall::Notify(n) => Some(n),
                NotifierCall::SetEnabled(_) | NotifierCall::Clear(_) => None,
            })
            .collect()
    }

    /// The notifications currently on the (imaginary) screen, oldest first.
    pub fn shown(&self) -> Vec<Notification> {
        self.shown.lock().clone()
    }

    /// Simulates a click on the `index`th shown notification: calls the activation handler with
    /// what the notification carries. `None` when there is no such notification or no handler.
    pub fn click(&self, index: usize) -> Option<NotificationActivation> {
        let activation = self.shown.lock().get(index)?.activation();
        let handler = self.handler.lock().clone()?;
        handler(activation.clone());
        Some(activation)
    }

    /// Simulates a click on the newest shown notification.
    pub fn click_last(&self) -> Option<NotificationActivation> {
        let last = self.shown.lock().len().checked_sub(1)?;
        self.click(last)
    }
}

#[async_trait]
impl NotifierPort for FakeNotifierPort {
    async fn set_enabled(&self, enabled: bool) -> OxiResult<NotifierStatus> {
        self.calls.record(NotifierCall::SetEnabled(enabled));
        let status = self.script.set_enabled.next_or_else(|| {
            Ok(if enabled {
                NotifierStatus::Ready
            } else {
                NotifierStatus::Off
            })
        })?;
        self.enabled.store(enabled, Ordering::SeqCst);
        if !enabled {
            self.shown.lock().clear();
        }
        Ok(status)
    }

    async fn notify(&self, notification: &Notification) -> OxiResult<()> {
        self.calls
            .record(NotifierCall::Notify(notification.clone()));
        self.script.notify.next_or_else(|| Ok(()))?;
        if self.is_enabled() {
            let mut shown = self.shown.lock();
            if let Some(tag) = &notification.tag {
                shown.retain(|n| n.tag.as_ref() != Some(tag));
            }
            shown.push(notification.clone());
        }
        Ok(())
    }

    async fn clear(&self, tag: &str) -> OxiResult<()> {
        self.calls.record(NotifierCall::Clear(tag.to_owned()));
        self.script.clear.next_or_else(|| Ok(()))?;
        self.shown.lock().retain(|n| n.tag.as_deref() != Some(tag));
        Ok(())
    }

    fn on_activated(&self, handler: ActivationHandler) {
        *self.handler.lock() = Some(handler);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::executor::block_on;
    use oxikube_domain::{ErrorKind, OxiError};
    use oxikube_ports::{NotificationLevel, unavailable};

    use super::*;

    fn note(tag: &str) -> Notification {
        Notification::new("Pod crashed", "web restarted", NotificationLevel::Warning).with_tag(tag)
    }

    #[test]
    fn starts_off_and_shows_nothing_until_enabled() {
        let fake = FakeNotifierPort::new();
        block_on(fake.notify(&note("a"))).unwrap();
        assert_eq!(fake.notifications().len(), 1);
        assert!(fake.shown().is_empty(), "off: recorded, not shown");

        assert_eq!(
            block_on(fake.set_enabled(true)).unwrap(),
            NotifierStatus::Ready
        );
        block_on(fake.notify(&note("a"))).unwrap();
        assert_eq!(fake.shown().len(), 1);

        assert_eq!(
            block_on(fake.set_enabled(false)).unwrap(),
            NotifierStatus::Off
        );
        assert!(fake.shown().is_empty());
        assert_eq!(
            fake.recorded_calls()[1],
            NotifierCall::SetEnabled(true),
            "calls are in order"
        );
    }

    #[test]
    fn scripted_errors_and_unavailable_status() {
        let fake = FakeNotifierPort::enabled();
        fake.script()
            .notify
            .push_err(OxiError::unsupported("no daemon"));
        assert_eq!(
            block_on(fake.notify(&note("a"))).unwrap_err().kind(),
            ErrorKind::Unsupported
        );
        assert!(fake.shown().is_empty());
        assert_eq!(
            fake.notifications().len(),
            1,
            "the refused call is recorded"
        );

        fake.script()
            .set_enabled
            .push_ok(NotifierStatus::Unavailable("no daemon".into()));
        assert_eq!(
            block_on(fake.set_enabled(true)).unwrap(),
            NotifierStatus::Unavailable("no daemon".into())
        );
        fake.script().set_enabled.push_err(unavailable("no bus"));
        assert!(block_on(fake.set_enabled(true)).is_err());
    }

    #[test]
    fn same_tag_replaces_and_clear_removes() {
        let fake = FakeNotifierPort::enabled();
        block_on(fake.notify(&note("a"))).unwrap();
        block_on(fake.notify(&note("b"))).unwrap();
        block_on(fake.notify(&note("a"))).unwrap();
        assert_eq!(fake.shown().len(), 2);
        block_on(fake.clear("a")).unwrap();
        assert_eq!(fake.shown(), vec![note("b")]);
        assert_eq!(
            fake.recorded_calls().last(),
            Some(&NotifierCall::Clear("a".into()))
        );
    }

    #[test]
    fn a_click_reaches_the_handler_with_the_target() {
        let fake = FakeNotifierPort::enabled();
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        fake.on_activated(Arc::new(move |a| sink.lock().push(a)));
        assert_eq!(fake.click_last(), None, "nothing shown yet");

        let target = "3f2a9c01b7d4e865/apps/v1/Deployment/prod/api"
            .parse()
            .unwrap();
        let shown = note("a").with_action_target(target);
        block_on(fake.notify(&shown)).unwrap();
        let clicked = fake.click_last().unwrap();
        assert_eq!(*got.lock(), vec![clicked.clone()]);
        assert_eq!(clicked, shown.activation());
    }
}
