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
    /// Platform id to what a click on it opens, for the notifications someone is waiting on. One
    /// waiter per id: a notification that replaces another in place (same id) hands its target
    /// to the waiter already there instead of starting a second one, which would see the same
    /// click and activate twice.
    live: HashMap<u32, NotificationActivation>,
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
                    live: HashMap::new(),
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

/// A thread waiting for the user to answer one notification.
struct Waiter {
    _slot: WaitSlot,
    /// The platform id the shared target is filed under, if the platform has ids.
    id: Option<u32>,
    /// The target of a notification without a platform id.
    own: Option<NotificationActivation>,
}

impl Inner {
    /// Decides who waits for the user's answer to a notification that was just shown. `None`
    /// when nobody needs to: no free slot, or a waiter on the same id already exists and now
    /// answers for this notification's target.
    fn waiter(
        &self,
        id: Option<u32>,
        activation: NotificationActivation,
        slot: Option<WaitSlot>,
    ) -> Option<Waiter> {
        let Some(id) = id else {
            return slot.map(|_slot| Waiter {
                _slot,
                id: None,
                own: Some(activation),
            });
        };
        let mut state = self.state.lock();
        if let Some(live) = state.live.get_mut(&id) {
            *live = activation;
            return None;
        }
        let _slot = slot?;
        state.live.insert(id, activation);
        Some(Waiter {
            _slot,
            id: Some(id),
            own: None,
        })
    }

    /// The user answered: a click runs the handler with the newest target filed for the id.
    fn answered(&self, waiter: Waiter, response: Response) {
        let activation = match waiter.id {
            Some(id) => self.state.lock().live.remove(&id),
            None => waiter.own,
        };
        if let (Response::Clicked, Some(activation)) = (response, activation) {
            self.activated(activation);
        }
    }

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
        let (replaces, shared_wait) = {
            let state = self.inner.state.lock();
            match &state.status {
                NotifierStatus::Off => return Ok(()),
                NotifierStatus::Unavailable(reason) => return Err(unavailable(reason.clone())),
                NotifierStatus::Ready => {
                    let replaces = notification
                        .tag
                        .as_ref()
                        .and_then(|tag| state.ids.get(tag).copied());
                    // The waiter on the id being replaced takes this notification's clicks too.
                    let shared = replaces.is_some_and(|id| state.live.contains_key(&id));
                    (replaces, shared)
                }
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
                let clickable = slot.is_some() || shared_wait;
                let shown = match inner.backend.show(&payload, clickable) {
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
                match inner.waiter(shown.id(), activation, slot) {
                    Some(waiter) => {
                        let response = shown.wait();
                        inner.answered(waiter, response);
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
