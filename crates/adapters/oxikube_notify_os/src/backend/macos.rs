//! macOS: `NSUserNotificationCenter` through `mac-notification-sys`.
//!
//! `notify-rust` wraps the same crate but never waits for a plain click (it only waits when an
//! action button is present), so the click-to-focus acceptance criterion needs the crate's own
//! `wait_for_click`. The modern `UNUserNotificationCenter` is `notify-rust`'s opt-in
//! `preview-macos-un` backend; it needs a signed bundle with an entitlement-free identifier and
//! its crate is not in this tree yet. Moving to it means replacing this file only.
//!
//! Identity: a process inside an `.app` bundle is attributed to the bundle's own identifier,
//! read once from the bundle's `Info.plist` when the user turns notifications on. An unbundled binary
//! is attributed to Terminal, which the OS may suppress; that is logged, not an error.

use std::sync::Once;

use mac_notification_sys as mac;

use super::{Backend, BackendError, Response, Shown, bundle_identifier};
use crate::payload::{Payload, Urgency};

/// Who a development build's notifications come from: the identity `mac-notification-sys` itself
/// falls back to when it cannot set another.
const DEV_IDENTITY: &str = "com.apple.Terminal";

/// The macOS backend. Holds no OS state: the notification centre is touched on first use.
pub(super) struct Macos {
    identified: Once,
}

impl Macos {
    pub(super) fn new() -> Self {
        Self {
            identified: Once::new(),
        }
    }

    /// Chooses the bundle identifier notifications are attributed to (once). Never asks the OS to
    /// look an application up: the crate's own default lookup shows a "Choose Application" dialog
    /// when it finds nothing, so an identity is always set here first.
    fn identify(&self) {
        self.identified.call_once(|| {
            let bundle = std::env::current_exe()
                .ok()
                .and_then(|exe| bundle_identifier(&exe));
            let id = bundle.as_deref().unwrap_or(DEV_IDENTITY);
            if bundle.is_none() {
                tracing::debug!(
                    "oxikube is not running from an .app bundle: OS notifications are attributed \
                     to {DEV_IDENTITY} and may be suppressed or not clickable"
                );
            }
            if let Err(error) = mac::set_application(id) {
                tracing::debug!(%error, bundle = %id, "could not set the notification identity");
            }
        });
    }
}

impl Backend for Macos {
    fn connect(&self) -> Result<(), BackendError> {
        self.identify();
        Ok(())
    }

    fn show(&self, payload: &Payload, clickable: bool) -> Result<Box<dyn Shown>, BackendError> {
        self.identify();
        Ok(Box::new(MacShown {
            payload: payload.clone(),
            clickable,
        }))
    }
}

/// A notification macOS has not been given yet: delivering it is what blocks.
struct MacShown {
    payload: Payload,
    clickable: bool,
}

impl MacShown {
    fn send(&self, wait: bool) -> Response {
        let mut notification = mac::Notification::new();
        notification
            .title(&self.payload.title)
            .message(&self.payload.body)
            .asynchronous(!wait)
            .wait_for_click(wait);
        if self.payload.urgency == Urgency::Normal {
            notification.default_sound();
        }
        match notification.send() {
            Ok(mac::NotificationResponse::Click) => Response::Clicked,
            Ok(_) => Response::Closed,
            Err(error) => {
                tracing::debug!(%error, "macOS did not deliver the notification");
                Response::Closed
            }
        }
    }
}

impl Shown for MacShown {
    fn id(&self) -> Option<u32> {
        None
    }

    fn wait(self: Box<Self>) -> Response {
        self.send(self.clickable)
    }

    fn detach(self: Box<Self>) {
        self.send(false);
    }
}
