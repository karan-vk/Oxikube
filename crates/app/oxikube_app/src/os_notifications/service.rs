//! [`OsNotificationService`]: the opt-in gate and the safe-text policy over a `NotifierPort`.

use std::sync::Arc;

use oxikube_ports::{
    Notification, NotificationLevel, NotifierPort, NotifierStatus, is_unavailable,
};
use parking_lot::Mutex;

use super::click::{ClickAction, ClickSink};

/// The tag of the test notification: sending it again replaces the one on screen.
const TEST_TAG: &str = "oxikube/test";

/// What became of a notification handed to [`OsNotificationService::notify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// Handed to the OS. (The OS may still suppress it: do-not-disturb, permission denied.)
    Sent,
    /// OS notifications are off: nothing was sent. The in-app notification is unaffected.
    Off,
    /// The system has no notification service. `hint` is a sentence to show the user, once; the
    /// later calls carry `None` so a missing daemon is explained a single time.
    Unavailable {
        /// The one-time explanation, `None` after the first call.
        hint: Option<String>,
    },
    /// The service refused or failed; the message is for the log.
    Failed(String),
}

/// Puts notifications on the desktop when, and only when, the user opted in. See the
/// [module docs](super).
///
/// Cheap to clone: a handle on shared state.
#[derive(Clone)]
pub struct OsNotificationService {
    inner: Arc<Inner>,
}

struct Inner {
    notifier: Arc<dyn NotifierPort>,
    state: Mutex<State>,
}

struct State {
    /// The setting: did the user opt in?
    wanted: bool,
    /// What the port said when it was last turned on or off.
    status: NotifierStatus,
    /// Whether the "no notification service" hint was handed out.
    hinted: bool,
}

impl OsNotificationService {
    /// A service over `notifier`, off until [`apply`](Self::apply) turns it on.
    pub fn new(notifier: Arc<dyn NotifierPort>) -> Self {
        Self {
            inner: Arc::new(Inner {
                notifier,
                state: Mutex::new(State {
                    wanted: false,
                    status: NotifierStatus::Off,
                    hinted: false,
                }),
            }),
        }
    }

    /// Whether the user opted in (the `events.os_notifications` setting).
    pub fn wanted(&self) -> bool {
        self.inner.state.lock().wanted
    }

    /// What the port reported when it was last switched.
    pub fn status(&self) -> NotifierStatus {
        self.inner.state.lock().status.clone()
    }

    /// Follows the setting: `true` turns the port on (the adapter connects to the OS and asks for
    /// permission now), `false` turns it off. Idempotent: a repeat of the current state neither
    /// reconnects nor asks again, except that a port that was `Unavailable` is tried again (the
    /// user may have started a daemon). Returns the resulting status; a port error becomes
    /// `Unavailable`, because the user's opt-in stands and a later toggle retries.
    pub async fn apply(&self, wanted: bool) -> NotifierStatus {
        {
            let mut state = self.inner.state.lock();
            if state.wanted == wanted && !matches!(state.status, NotifierStatus::Unavailable(_)) {
                return state.status.clone();
            }
            state.wanted = wanted;
            if !wanted {
                state.status = NotifierStatus::Off;
                state.hinted = false;
            }
        }
        let result = self.inner.notifier.set_enabled(wanted).await;
        let status = match (wanted, result) {
            (false, _) => NotifierStatus::Off,
            (true, Ok(status)) => status,
            (true, Err(error)) => NotifierStatus::Unavailable(error.message().to_owned()),
        };
        let mut state = self.inner.state.lock();
        // The user may have flipped the setting again while the port was answering: the newest
        // wish wins, and the port gets its own `apply` for it.
        if state.wanted == wanted {
            state.status = status.clone();
            state.hinted = false;
        }
        status
    }

    /// Shows `notification` on the desktop if the user opted in. The text is redacted and capped
    /// first. Never panics and never fails the caller: the outcome is a [`Delivery`].
    pub async fn notify(&self, notification: &Notification) -> Delivery {
        match self.gate() {
            Gate::Off => return Delivery::Off,
            Gate::Unavailable(reason) => return self.unavailable(&reason),
            Gate::Open => {}
        }
        match self.inner.notifier.notify(&notification.sanitised()).await {
            Ok(()) => Delivery::Sent,
            Err(error) if is_unavailable(&error) => {
                let reason = error.message().to_owned();
                self.inner.state.lock().status = NotifierStatus::Unavailable(reason.clone());
                self.unavailable(&reason)
            }
            Err(error) => {
                tracing::warn!(%error, "the OS notifier failed");
                Delivery::Failed(error.message().to_owned())
            }
        }
    }

    /// Shows the test notification (`app::SendTestNotification`). Clicking it only brings the
    /// window to the front.
    pub async fn send_test(&self) -> Delivery {
        let test = Notification::new(
            "Oxikube test notification",
            "OS notifications work. Click to bring Oxikube to the front.",
            NotificationLevel::Info,
        )
        .with_tag(TEST_TAG);
        self.notify(&test).await
    }

    /// Sets where clicks on shown notifications go. The sink runs on an adapter thread.
    pub fn on_click(&self, sink: ClickSink) {
        self.inner
            .notifier
            .on_activated(Arc::new(move |activation| {
                sink(ClickAction::from(activation))
            }));
    }

    fn gate(&self) -> Gate {
        let state = self.inner.state.lock();
        if !state.wanted {
            return Gate::Off;
        }
        match &state.status {
            NotifierStatus::Unavailable(reason) => Gate::Unavailable(reason.clone()),
            NotifierStatus::Off | NotifierStatus::Ready => Gate::Open,
        }
    }

    fn unavailable(&self, reason: &str) -> Delivery {
        let mut state = self.inner.state.lock();
        let hint = (!state.hinted).then(|| {
            state.hinted = true;
            unavailable_notice(reason)
        });
        Delivery::Unavailable { hint }
    }
}

enum Gate {
    Off,
    Unavailable(String),
    Open,
}

/// The sentence telling the user the opt-in stands but this system cannot show OS notifications.
pub(super) fn unavailable_notice(reason: &str) -> String {
    format!(
        "OS notifications are on, but this system cannot show them ({reason}). \
         Notifications still appear in Oxikube."
    )
}
