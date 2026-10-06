//! The per-feed task: open the planned feed, apply its batches to the entry, and reopen with
//! backoff when it fails retryably or ends.
//!
//! ```text
//! Idle ──first subscriber──▶ Warming ──initial list──▶ Ready ◀──▶ delta batches
//!                              │  ▲                      │
//!            terminal error ◀──┘  └── retry (backoff) ◀──┤ retryable error / feed ended
//!          (Forbidden/Failed;                             │
//!           next subscriber restarts)      last subscriber dropped ──▶ Grace ──timer──▶ aborted
//! ```
//!
//! The task holds only a `Weak` to its entry; the entry holds the task's abort-on-drop guard,
//! so tearing the entry down aborts the task and drops the port's stream with it.

use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::StreamExt;
use oxikube_ports::ClockPort;

use super::delta::FeedState;
use super::entry::FeedEntry;
use super::feed::{StorePorts, open};
use super::object::FeedKey;
use super::policy::FeedKind;

/// Doubling retry delay.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Backoff {
    initial: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    pub fn new(initial: Duration, max: Duration) -> Self {
        Self {
            initial,
            max,
            next: initial,
        }
    }

    fn reset(&mut self) {
        self.next = self.initial;
    }

    fn take(&mut self) -> Duration {
        let delay = self.next;
        self.next = (self.next * 2).min(self.max);
        delay
    }
}

/// Runs one feed until it fails terminally or the entry is gone (or the task is aborted).
pub(crate) async fn drive(
    entry: Weak<FeedEntry>,
    ports: StorePorts,
    kind: FeedKind,
    key: FeedKey,
    clock: Arc<dyn ClockPort>,
    mut backoff: Backoff,
) {
    loop {
        match open(&ports, kind, &key).await {
            Ok(mut feed) => {
                while let Some(item) = feed.next().await {
                    let Some(live) = entry.upgrade() else { return };
                    match item {
                        Ok(batch) => {
                            live.apply(batch);
                            backoff.reset();
                        }
                        Err(error) => {
                            let retrying = error.is_retryable();
                            live.set_state(FeedState::from_error(&error, retrying));
                            if !retrying {
                                live.stopped();
                                return;
                            }
                        }
                    }
                }
                let Some(live) = entry.upgrade() else { return };
                live.set_state(FeedState::Retrying {
                    message: format!("the feed for {key} ended; reopening"),
                });
            }
            Err(error) => {
                let Some(live) = entry.upgrade() else { return };
                let retrying = error.is_retryable();
                live.set_state(FeedState::from_error(&error, retrying));
                if !retrying {
                    live.stopped();
                    return;
                }
            }
        }
        clock.sleep(backoff.take()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_to_the_cap_and_resets() {
        let mut b = Backoff::new(Duration::from_secs(1), Duration::from_secs(5));
        let delays: Vec<u64> = (0..5).map(|_| b.take().as_secs()).collect();
        assert_eq!(delays, vec![1, 2, 4, 5, 5]);
        b.reset();
        assert_eq!(b.take(), Duration::from_secs(1));
    }
}
