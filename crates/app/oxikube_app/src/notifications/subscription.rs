//! [`NotificationsSubscription`]: what the centre's views and the OS output are told, coalesced
//! per poll.

use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::task::{Context, Poll, Waker};

use futures::Stream;
use oxikube_domain::ids::ClusterId;
use parking_lot::Mutex;

use super::model::Notification;
use super::service::Inner;

/// The most raised notifications one item carries; past it [`NotificationsChanged::more_raised`]
/// counts the rest (a burst never builds an unbounded list for a view that polls once a frame).
pub const MAX_RAISED_PER_ITEM: usize = 32;

/// What changed in the centre since a subscriber last polled.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NotificationsChanged {
    /// The list, the unread count or the muted clusters changed: read them again. Always set on
    /// a subscription's first item.
    pub changed: bool,
    /// The notifications that should toast (and reach the OS) now, oldest first, at most
    /// [`MAX_RAISED_PER_ITEM`].
    pub raised: Vec<Arc<Notification>>,
    /// How many more were raised than `raised` holds.
    pub more_raised: usize,
}

impl NotificationsChanged {
    /// Whether there is nothing to tell.
    pub fn is_empty(&self) -> bool {
        !self.changed && self.raised.is_empty() && self.more_raised == 0
    }
}

/// One subscriber's pending changes and waker.
#[derive(Debug)]
pub(crate) struct Mailbox(Mutex<Slot>);

#[derive(Debug)]
struct Slot {
    pending: NotificationsChanged,
    waker: Option<Waker>,
}

impl Mailbox {
    pub(crate) fn new() -> Self {
        Self(Mutex::new(Slot {
            pending: NotificationsChanged {
                changed: true,
                ..NotificationsChanged::default()
            },
            waker: None,
        }))
    }

    /// Adds a change (and a raised notification); returns the waker to wake once the caller's
    /// locks are released.
    pub(crate) fn push(&self, raised: Option<&Arc<Notification>>) -> Option<Waker> {
        let mut slot = self.0.lock();
        slot.pending.changed = true;
        if let Some(raised) = raised {
            if slot.pending.raised.len() < MAX_RAISED_PER_ITEM {
                slot.pending.raised.push(raised.clone());
            } else {
                slot.pending.more_raised += 1;
            }
        }
        slot.waker.take()
    }

    fn poll(&self, cx: &mut Context<'_>) -> Poll<NotificationsChanged> {
        let mut slot = self.0.lock();
        if slot.pending.is_empty() {
            slot.waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        Poll::Ready(std::mem::take(&mut slot.pending))
    }
}

/// A live view of the notification centre: a [`Stream`] of [`NotificationsChanged`], one item
/// per poll however many notifications arrived in between. The first item says `changed`. The
/// stream does not end while the service lives; dropping it unsubscribes.
pub struct NotificationsSubscription {
    pub(crate) inner: Weak<Inner>,
    pub(crate) id: u64,
    pub(crate) mailbox: Arc<Mailbox>,
}

impl std::fmt::Debug for NotificationsSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationsSubscription")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl NotificationsSubscription {
    /// Whether `cluster`'s notifications may reach the desktop under the rules in effect now
    /// (`events.os_notifications` of the cluster; `true` once the service is gone).
    pub fn os_allowed(&self, cluster: &ClusterId) -> bool {
        self.inner
            .upgrade()
            .is_none_or(|inner| inner.state.lock().rules.os_allowed(cluster))
    }
}

impl Stream for NotificationsSubscription {
    type Item = NotificationsChanged;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.mailbox.poll(cx).map(Some)
    }
}

impl Drop for NotificationsSubscription {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            inner.state.lock().subscribers.remove(&self.id);
        }
    }
}
