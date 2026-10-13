//! [`OsNotifier`]: the `NotifierPort` over a platform [`Backend`].

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use futures::channel::oneshot;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    ActivationHandler, Notification, NotificationActivation, NotifierPort, NotifierStatus,
    unavailable,
};
use parking_lot::Mutex;

use crate::backend::{self, Backend, BackendError, Response};
use crate::payload::Payload;

/// The most threads waiting for a click at once. A notification beyond that is shown without
/// click-through, so a flood of unread notifications cannot pile up threads.
pub const MAX_WAITING: usize = 16;

/// The most tags remembered for replacement.
const MAX_TAGS: usize = 256;

/// Shows notifications on the platform notification centre once the user turned them on.
///
/// Cheap to clone-by-`Arc` and to build: nothing touches the OS until
/// [`set_enabled(true)`](NotifierPort::set_enabled).
pub struct OsNotifier {
    inner: Arc<Inner>,
}

struct Inner {
    backend: Arc<dyn Backend>,
    state: Mutex<State>,
    waiting: AtomicUsize,
}

struct State {
    status: NotifierStatus,
    handler: Option<ActivationHandler>,
    /// Tag to the platform id of the notification showing it (for in-place replacement).
    ids: HashMap<String, u32>,
}

impl OsNotifier {
    /// A notifier on the backend of this platform, off until the user turns it on.
    pub fn new() -> Self {
        Self::with_backend(backend::platform())
    }

    /// A notifier on `backend` (tests use a scripted one).
    pub fn with_backend(backend: Arc<dyn Backend>) -> Self {
        Self {
            inner: Arc::new(Inner {
                backend,
                state: Mutex::new(State {
                    status: NotifierStatus::Off,
                    handler: None,
                    ids: HashMap::new(),
                }),
                waiting: AtomicUsize::new(0),
            }),
        }
    }

    /// Where the notifier stands now.
    pub fn status(&self) -> NotifierStatus {
        self.inner.state.lock().status.clone()
    }
}

impl Default for OsNotifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Runs `work` on a thread of its own and awaits its result.
async fn blocking<T: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> T + Send + 'static,
) -> OxiResult<T> {
    let (tx, rx) = oneshot::channel();
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            let _ = tx.send(work());
        })
        .map_err(|e| OxiError::internal(format!("could not start the {name} thread: {e}")))?;
    rx.await
        .map_err(|_| OxiError::internal(format!("the {name} thread ended without answering")))
}

fn map_error(error: BackendError) -> OxiError {
    match error {
        BackendError::Unavailable(reason) => unavailable(reason),
        BackendError::Failed(message) => OxiError::internal(message),
    }
}

/// Counts a waiting thread; the slot is freed when it ends.
struct WaitSlot(Arc<Inner>);

impl WaitSlot {
    fn take(inner: &Arc<Inner>) -> Option<Self> {
        inner
            .waiting
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_WAITING).then_some(n + 1)
            })
            .ok()
            .map(|_| Self(inner.clone()))
    }
}

impl Drop for WaitSlot {
    fn drop(&mut self) {
        self.0.waiting.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Inner {
    fn activated(&self, activation: NotificationActivation) {
        let handler = self.state.lock().handler.clone();
        if let Some(handler) = handler {
            handler(activation);
        }
    }

    fn remember(&self, tag: &str, id: u32) {
        let mut state = self.state.lock();
        if state.ids.len() >= MAX_TAGS && !state.ids.contains_key(tag) {
            state.ids.clear();
        }
        state.ids.insert(tag.to_owned(), id);
    }
}

#[async_trait]
impl NotifierPort for OsNotifier {
    async fn set_enabled(&self, enabled: bool) -> OxiResult<NotifierStatus> {
        if !enabled {
            let mut state = self.inner.state.lock();
            state.status = NotifierStatus::Off;
            state.ids.clear();
            return Ok(NotifierStatus::Off);
        }
        if self.status() == NotifierStatus::Ready {
            return Ok(NotifierStatus::Ready);
        }
        let backend = self.inner.backend.clone();
        let status = match blocking("oxikube-notify-connect", move || backend.connect()).await? {
            Ok(()) => NotifierStatus::Ready,
            Err(BackendError::Unavailable(reason) | BackendError::Failed(reason)) => {
                NotifierStatus::Unavailable(reason)
            }
        };
        self.inner.state.lock().status = status.clone();
        Ok(status)
    }

    async fn notify(&self, notification: &Notification) -> OxiResult<()> {
        let replaces = {
            let state = self.inner.state.lock();
            match &state.status {
                NotifierStatus::Off => return Ok(()),
                NotifierStatus::Unavailable(reason) => return Err(unavailable(reason.clone())),
                NotifierStatus::Ready => notification
                    .tag
                    .as_ref()
                    .and_then(|tag| state.ids.get(tag).copied()),
            }
        };
        let payload = Payload::of(notification, replaces);
        let activation = notification.activation();
        let slot = WaitSlot::take(&self.inner);
        let inner = self.inner.clone();
        let (shown_tx, shown_rx) = oneshot::channel();
        // The thread outlives this call when it waits for a click.
        std::thread::Builder::new()
            .name("oxikube-notify".to_owned())
            .spawn(move || {
                let shown = match inner.backend.show(&payload, slot.is_some()) {
                    Ok(shown) => shown,
                    Err(error) => {
                        let _ = shown_tx.send(Err(error));
                        return;
                    }
                };
                if let (Some(tag), Some(id)) = (&activation.tag, shown.id()) {
                    inner.remember(tag, id);
                }
                let _ = shown_tx.send(Ok(()));
                match slot {
                    Some(_slot) => {
                        if shown.wait() == Response::Clicked {
                            inner.activated(activation);
                        }
                    }
                    None => shown.detach(),
                }
            })
            .map_err(|e| OxiError::internal(format!("could not start a notify thread: {e}")))?;
        match shown_rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                if let BackendError::Unavailable(reason) = &error {
                    self.inner.state.lock().status = NotifierStatus::Unavailable(reason.clone());
                }
                Err(map_error(error))
            }
            Err(_) => Err(OxiError::internal(
                "the notify thread ended without answering",
            )),
        }
    }

    async fn clear(&self, tag: &str) -> OxiResult<()> {
        // Neither platform API this adapter uses can withdraw a shown notification: forget the
        // tag so the next notification with it is a fresh one.
        self.inner.state.lock().ids.remove(tag);
        Ok(())
    }

    fn on_activated(&self, handler: ActivationHandler) {
        self.inner.state.lock().handler = Some(handler);
    }
}

#[cfg(test)]
mod tests;
