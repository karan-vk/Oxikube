//! [`FakeClockPort`]: a virtual clock the test advances by hand.

use std::time::Duration;

use async_trait::async_trait;
use futures::channel::oneshot;
use jiff::Timestamp;
use oxikube_ports::ClockPort;
use parking_lot::Mutex;

use crate::script::CallLog;

/// The instant a [`FakeClockPort::default`] starts at: `2026-01-01T00:00:00Z`.
pub const DEFAULT_START: &str = "2026-01-01T00:00:00Z";

/// One call made on a [`FakeClockPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClockCall {
    /// `sleep(duration)`.
    Sleep(Duration),
    /// `sleep_until(deadline)`.
    SleepUntil(Timestamp),
}

struct Sleeper {
    deadline: Timestamp,
    wake: oneshot::Sender<()>,
}

struct ClockState {
    now: Timestamp,
    sleepers: Vec<Sleeper>,
}

/// A [`ClockPort`] whose time only moves when the test calls [`advance`](Self::advance)
/// or [`set`](Self::set).
///
/// `sleep` and `sleep_until` return futures that complete once virtual time reaches
/// their deadline (immediately when it already has). They are woken through
/// `futures` oneshot channels, so they work under any executor, including GPUI's test
/// scheduler (`advance_clock` + `run_until_parked`), and never start an OS thread or
/// sleep for real. There is nothing to script: the clock's "responses" are the times
/// the test sets.
pub struct FakeClockPort {
    state: Mutex<ClockState>,
    calls: CallLog<ClockCall>,
}

impl Default for FakeClockPort {
    fn default() -> Self {
        // DEFAULT_START is a valid RFC 3339 literal, checked by a unit test.
        Self::new(DEFAULT_START.parse().unwrap_or(Timestamp::UNIX_EPOCH))
    }
}

impl std::fmt::Debug for FakeClockPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock();
        f.debug_struct("FakeClockPort")
            .field("now", &state.now)
            .field("sleepers", &state.sleepers.len())
            .finish()
    }
}

impl FakeClockPort {
    /// A clock frozen at `start`.
    pub fn new(start: Timestamp) -> Self {
        Self {
            state: Mutex::new(ClockState {
                now: start,
                sleepers: Vec::new(),
            }),
            calls: CallLog::default(),
        }
    }

    /// Moves virtual time forward by `by` and wakes every sleeper whose deadline passed.
    pub fn advance(&self, by: Duration) {
        let mut state = self.state.lock();
        let to = state.now.checked_add(by).unwrap_or(Timestamp::MAX);
        Self::move_to(&mut state, to);
    }

    /// Sets virtual time to `to` (never backwards: an earlier instant is ignored) and
    /// wakes every sleeper whose deadline passed.
    pub fn set(&self, to: Timestamp) {
        let mut state = self.state.lock();
        if to > state.now {
            Self::move_to(&mut state, to);
        }
    }

    /// Jumps to the earliest pending deadline and wakes its sleepers. Returns that
    /// deadline, or `None` when nothing is sleeping.
    pub fn advance_to_next(&self) -> Option<Timestamp> {
        let mut state = self.state.lock();
        let next = state.sleepers.iter().map(|s| s.deadline).min()?;
        let to = next.max(state.now);
        Self::move_to(&mut state, to);
        Some(to)
    }

    /// Number of sleeps still waiting for virtual time to reach their deadline.
    pub fn pending_sleepers(&self) -> usize {
        let mut state = self.state.lock();
        state.sleepers.retain(|s| !s.wake.is_canceled());
        state.sleepers.len()
    }

    /// Every call made on this clock so far, in call order.
    pub fn recorded_calls(&self) -> Vec<ClockCall> {
        self.calls.calls()
    }

    /// Forgets the recorded calls.
    pub fn clear_calls(&self) {
        self.calls.clear();
    }

    fn move_to(state: &mut ClockState, to: Timestamp) {
        state.now = to;
        let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut state.sleepers)
            .into_iter()
            .partition(|s| s.deadline <= to);
        state.sleepers = waiting;
        for sleeper in due {
            // The sleeping future may have been dropped; nothing to wake then.
            let _ = sleeper.wake.send(());
        }
    }

    async fn wait_until(&self, deadline: Timestamp) {
        let wake = {
            let mut state = self.state.lock();
            if deadline <= state.now {
                return;
            }
            let (tx, rx) = oneshot::channel();
            state.sleepers.push(Sleeper { deadline, wake: tx });
            rx
        };
        // Err only when the clock is dropped; treat that as "time is up".
        let _ = wake.await;
    }
}

#[async_trait]
impl ClockPort for FakeClockPort {
    fn now(&self) -> Timestamp {
        self.state.lock().now
    }

    async fn sleep(&self, duration: Duration) {
        self.calls.record(ClockCall::Sleep(duration));
        let deadline = self.now().checked_add(duration).unwrap_or(Timestamp::MAX);
        self.wait_until(deadline).await;
    }

    async fn sleep_until(&self, deadline: Timestamp) {
        self.calls.record(ClockCall::SleepUntil(deadline));
        self.wait_until(deadline).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use futures::executor::block_on;

    #[test]
    fn default_start_parses() {
        let expected: Timestamp = DEFAULT_START.parse().unwrap();
        assert_eq!(FakeClockPort::default().now(), expected);
    }

    #[test]
    fn sleep_completes_only_when_time_is_advanced() {
        let clock = FakeClockPort::default();
        let start = clock.now();
        let mut sleep = clock.sleep(Duration::from_secs(5)).boxed();
        assert!((&mut sleep).now_or_never().is_none());
        assert_eq!(clock.pending_sleepers(), 1);
        clock.advance(Duration::from_secs(4));
        assert!((&mut sleep).now_or_never().is_none());
        clock.advance(Duration::from_secs(1));
        assert!(sleep.now_or_never().is_some());
        assert_eq!(clock.pending_sleepers(), 0);
        assert_eq!(
            clock.now(),
            start.checked_add(Duration::from_secs(5)).unwrap()
        );
        assert_eq!(
            clock.recorded_calls(),
            vec![ClockCall::Sleep(Duration::from_secs(5))]
        );
    }

    #[test]
    fn past_deadlines_return_immediately_and_set_never_goes_back() {
        let clock = FakeClockPort::default();
        let start = clock.now();
        block_on(clock.sleep_until(start));
        block_on(clock.sleep(Duration::ZERO));
        clock.set(Timestamp::UNIX_EPOCH);
        assert_eq!(clock.now(), start);
        assert_eq!(clock.recorded_calls().len(), 2);
        clock.clear_calls();
        assert!(clock.recorded_calls().is_empty());
    }

    #[test]
    fn advance_to_next_jumps_to_the_earliest_deadline() {
        let clock = FakeClockPort::default();
        let start = clock.now();
        let mut a = clock.sleep(Duration::from_secs(10)).boxed();
        let mut b = clock.sleep(Duration::from_secs(3)).boxed();
        assert!((&mut a).now_or_never().is_none());
        assert!((&mut b).now_or_never().is_none());
        let first = clock.advance_to_next().unwrap();
        assert_eq!(first, start.checked_add(Duration::from_secs(3)).unwrap());
        assert!(b.now_or_never().is_some());
        assert!((&mut a).now_or_never().is_none());
        clock.advance_to_next();
        assert!(a.now_or_never().is_some());
        assert_eq!(clock.advance_to_next(), None);
    }
}
