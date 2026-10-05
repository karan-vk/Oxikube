//! Publishing a forward's [`ForwardStatus`]: the latest value for polling and every
//! transition for observers that must not miss one (`TargetGone` followed by recovery).

use std::net::SocketAddr;
use std::sync::Arc;

use oxikube_domain::{ForwardStatus, OxiError};
use tokio::sync::{broadcast, watch};

/// Transitions kept for a slow subscriber before it sees `Lagged`.
const EVENT_BACKLOG: usize = 64;

struct Inner {
    latest: watch::Sender<ForwardStatus>,
    events: broadcast::Sender<ForwardStatus>,
}

/// Shared by the session task, the bridges and the handle. Clones share state.
#[derive(Clone)]
pub(super) struct StatusHub(Arc<Inner>);

impl StatusHub {
    pub(super) fn new() -> Self {
        Self(Arc::new(Inner {
            latest: watch::channel(ForwardStatus::Starting).0,
            events: broadcast::channel(EVENT_BACKLOG).0,
        }))
    }

    pub(super) fn latest(&self) -> ForwardStatus {
        self.0.latest.borrow().clone()
    }

    pub(super) fn watch(&self) -> watch::Receiver<ForwardStatus> {
        self.0.latest.subscribe()
    }

    pub(super) fn subscribe(&self) -> broadcast::Receiver<ForwardStatus> {
        self.0.events.subscribe()
    }

    /// Records `status` as the latest and announces it. Repeating the latest status is a no-op.
    pub(super) fn publish(&self, status: ForwardStatus) {
        let changed = self.0.latest.send_if_modified(|latest| {
            let changed = *latest != status;
            if changed {
                latest.clone_from(&status);
            }
            changed
        });
        if changed {
            // No subscriber is not an error.
            let _ = self.0.events.send(status);
        }
    }

    /// Announces a failure; the forward keeps running.
    pub(super) fn error(&self, err: &OxiError) {
        self.publish(ForwardStatus::Error {
            kind: err.kind(),
            message: err.message().to_owned(),
        });
    }

    /// Goes back to `Listening` when the latest status is an `Error`: a connection that
    /// works again clears the failure.
    pub(super) fn recover(&self, local_addr: SocketAddr, pod: &str) {
        if matches!(self.latest(), ForwardStatus::Error { .. }) {
            self.publish(ForwardStatus::Listening {
                local_addr,
                pod: pod.to_owned(),
            });
        }
    }
}

impl std::fmt::Debug for StatusHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("StatusHub").field(&self.latest()).finish()
    }
}

/// Publishes `Stopped` when dropped. The session task owns one, so the last status is
/// `Stopped` however the task ends: it returns, or the handle drops and aborts it.
pub(super) struct StopGuard(pub(super) StatusHub);

impl Drop for StopGuard {
    fn drop(&mut self) {
        self.0.publish(ForwardStatus::Stopped);
    }
}
