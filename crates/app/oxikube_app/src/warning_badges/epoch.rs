//! [`Epoch`] and [`BadgeChanges`]: "the badges changed", coalesced.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use futures::Stream;
use parking_lot::Mutex;

#[derive(Default)]
struct State {
    version: u64,
    wakers: Vec<Waker>,
}

/// A version counter with wakers: every change bumps it; a [`BadgeChanges`] yields once per
/// bump it has not seen.
#[derive(Default)]
pub(crate) struct Epoch(Arc<Mutex<State>>);

impl Epoch {
    pub(crate) fn bump(&self) {
        let wakers = {
            let mut state = self.0.lock();
            state.version += 1;
            std::mem::take(&mut state.wakers)
        };
        for waker in wakers {
            waker.wake();
        }
    }

    pub(crate) fn changes(&self) -> BadgeChanges {
        let seen = self.0.lock().version;
        BadgeChanges {
            state: self.0.clone(),
            seen,
        }
    }
}

/// See [`WarningBadges::changes`](super::WarningBadges::changes).
pub struct BadgeChanges {
    state: Arc<Mutex<State>>,
    seen: u64,
}

impl Stream for BadgeChanges {
    type Item = ();

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<()>> {
        let mut state = self.state.lock();
        if state.version == self.seen {
            if !state.wakers.iter().any(|w| w.will_wake(cx.waker())) {
                state.wakers.push(cx.waker().clone());
            }
            return Poll::Pending;
        }
        let version = state.version;
        drop(state);
        self.seen = version;
        Poll::Ready(Some(()))
    }
}

#[cfg(test)]
mod tests {
    use futures::{FutureExt as _, StreamExt as _};

    use super::*;

    #[test]
    fn a_burst_of_bumps_is_one_item() {
        let epoch = Epoch::default();
        let mut changes = epoch.changes();
        assert!(changes.next().now_or_never().is_none(), "nothing yet");
        for _ in 0..1_000 {
            epoch.bump();
        }
        assert_eq!(changes.next().now_or_never(), Some(Some(())));
        assert!(
            changes.next().now_or_never().is_none(),
            "one item, not 1000"
        );
        epoch.bump();
        assert_eq!(changes.next().now_or_never(), Some(Some(())));
    }

    #[test]
    fn a_stream_starts_at_the_current_version() {
        let epoch = Epoch::default();
        epoch.bump();
        let mut changes = epoch.changes();
        assert!(changes.next().now_or_never().is_none());
    }
}
