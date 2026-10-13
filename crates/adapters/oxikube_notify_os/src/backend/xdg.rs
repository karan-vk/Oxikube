//! Linux and the BSDs: freedesktop notifications over D-Bus through `notify-rust` (zbus).
//!
//! The `default` action is what GNOME and KDE invoke when the notification body is clicked; it
//! is added only when the server advertises the `actions` capability (otherwise the
//! notification is plain and cannot be clicked through). A notification whose id was passed in
//! [`Payload::replaces`] replaces that one in place.

use std::sync::atomic::{AtomicBool, Ordering};

use notify_rust::{Notification, NotificationHandle, NotificationResponse, Urgency as XdgUrgency};

use super::{Backend, BackendError, Response, Shown};
use crate::payload::{Payload, Urgency};

/// The freedesktop backend.
pub(super) struct Xdg {
    actions: AtomicBool,
}

impl Xdg {
    pub(super) fn new() -> Self {
        Self {
            actions: AtomicBool::new(false),
        }
    }
}

fn unavailable(error: &notify_rust::error::Error) -> BackendError {
    BackendError::Unavailable(format!(
        "the D-Bus notification service did not answer ({error})"
    ))
}

impl Backend for Xdg {
    fn connect(&self) -> Result<(), BackendError> {
        let capabilities = notify_rust::get_capabilities().map_err(|e| unavailable(&e))?;
        self.actions.store(
            capabilities
                .iter()
                .any(|capability| capability == "actions"),
            Ordering::Relaxed,
        );
        Ok(())
    }

    fn show(&self, payload: &Payload, clickable: bool) -> Result<Box<dyn Shown>, BackendError> {
        let mut notification = Notification::new();
        notification
            .appname("Oxikube")
            .summary(&payload.title)
            .body(&payload.body)
            .icon("oxikube")
            .urgency(match payload.urgency {
                Urgency::Low => XdgUrgency::Low,
                Urgency::Normal => XdgUrgency::Normal,
            });
        if let Some(id) = payload.replaces {
            notification.id(id);
        }
        let click_through = clickable && self.actions.load(Ordering::Relaxed);
        if click_through {
            notification.action("default", "Open");
        }
        let handle = notification.show().map_err(|e| unavailable(&e))?;
        Ok(Box::new(XdgShown {
            id: handle.id(),
            handle,
            click_through,
        }))
    }
}

struct XdgShown {
    id: u32,
    handle: NotificationHandle,
    click_through: bool,
}

impl Shown for XdgShown {
    fn id(&self) -> Option<u32> {
        Some(self.id)
    }

    fn wait(self: Box<Self>) -> Response {
        if !self.click_through {
            return Response::Closed;
        }
        let mut response = Response::Closed;
        let waited = self
            .handle
            .wait_for_response(|answer: &NotificationResponse| {
                if matches!(
                    answer,
                    NotificationResponse::Default | NotificationResponse::Action(_)
                ) {
                    response = Response::Clicked;
                }
            });
        if let Err(error) = waited {
            tracing::debug!(%error, "lost the notification service while waiting for a click");
        }
        response
    }

    fn detach(self: Box<Self>) {}
}
