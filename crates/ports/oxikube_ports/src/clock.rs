//! [`ClockPort`]: wall-clock time and sleeping, injectable so tests run on a virtual clock.
//!
//! # Adapter
//!
//! The production implementation lives in `oxikube_runtime` (the platform layer owns
//! the async runtime): `now()` reads the system clock and `sleep` is the runtime's
//! timer. `oxikube_testkit` ships `FakeClockPort`, whose time only moves when the
//! test advances it. Services in `oxikube_app` take an `Arc<dyn ClockPort>` instead
//! of calling `Timestamp::now()` or a runtime sleep, and GPUI tests must never use
//! real sleeps (`references/testing.md`).

use std::time::Duration;

use async_trait::async_trait;
use jiff::Timestamp;

/// Source of the current time and of timers.
#[async_trait]
pub trait ClockPort: Send + Sync {
    /// The current wall-clock time. Cheap and non-blocking.
    fn now(&self) -> Timestamp;

    /// Completes after `duration` has elapsed on this clock. A virtual clock
    /// completes it when the test advances time far enough.
    async fn sleep(&self, duration: Duration);

    /// Completes once [`now`](Self::now) reaches `deadline`; returns at once if it
    /// already has.
    async fn sleep_until(&self, deadline: Timestamp) {
        let remaining = deadline.duration_since(self.now());
        if let Ok(remaining) = Duration::try_from(remaining) {
            if !remaining.is_zero() {
                self.sleep(remaining).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A clock whose `sleep` advances its own time instantly and records the request.
    struct Virtual {
        now: Mutex<Timestamp>,
        slept: Mutex<Vec<Duration>>,
    }

    #[async_trait]
    impl ClockPort for Virtual {
        fn now(&self) -> Timestamp {
            *self.now.lock().unwrap()
        }

        async fn sleep(&self, duration: Duration) {
            self.slept.lock().unwrap().push(duration);
            let mut now = self.now.lock().unwrap();
            *now = now.checked_add(duration).unwrap();
        }
    }

    #[test]
    fn sleep_until_sleeps_only_the_remaining_time() {
        let start = Timestamp::from_second(1_000).unwrap();
        let clock = Arc::new(Virtual {
            now: Mutex::new(start),
            slept: Mutex::default(),
        });
        let dyn_clock: Arc<dyn ClockPort> = clock.clone();
        futures::executor::block_on(async {
            dyn_clock
                .sleep_until(Timestamp::from_second(1_005).unwrap())
                .await;
            // Deadline in the past: no sleep.
            dyn_clock.sleep_until(start).await;
        });
        assert_eq!(*clock.slept.lock().unwrap(), vec![Duration::from_secs(5)]);
        assert_eq!(dyn_clock.now(), Timestamp::from_second(1_005).unwrap());
    }
}
