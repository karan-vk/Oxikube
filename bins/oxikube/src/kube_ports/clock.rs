//! [`SystemClock`]: the app's [`ClockPort`], the wall clock and Tokio's timer.

use std::time::Duration;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_ports::ClockPort;
use tokio::runtime::Handle;

/// `now()` is the system clock; `sleep` is Tokio's timer.
///
/// Sleeps are awaited on whichever executor polls them: on the Tokio bridge (`spawn_kube`) the
/// timer runs in place, anywhere else (a GPUI task) it runs on `handle` and the caller awaits its
/// end, so a sleep never needs a Tokio context of its own.
#[derive(Debug, Clone)]
pub struct SystemClock {
    handle: Handle,
}

impl SystemClock {
    /// A clock whose timers run on `handle` when the caller is not on Tokio.
    pub fn new(handle: Handle) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl ClockPort for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    async fn sleep(&self, duration: Duration) {
        if Handle::try_current().is_ok() {
            tokio::time::sleep(duration).await;
        } else {
            // The timer is created inside the task: it needs the runtime's context. A failed join
            // is a runtime shutting down: the sleep is over either way.
            let timer = async move { tokio::time::sleep(duration).await };
            let _ = self.handle.spawn(timer).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sleep_outside_tokio_runs_on_the_handle() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let clock = SystemClock::new(runtime.handle().clone());
        let worker = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            futures::executor::block_on(clock.sleep(Duration::from_millis(5)));
            started.elapsed()
        });
        // The current-thread runtime drives its timer only while something blocks on it.
        let elapsed = runtime.block_on(async {
            loop {
                if worker.is_finished() {
                    break worker.join().unwrap();
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });
        assert!(elapsed >= Duration::from_millis(5), "{elapsed:?}");
    }

    #[tokio::test]
    async fn a_sleep_on_tokio_uses_the_timer_in_place() {
        let clock = SystemClock::new(Handle::current());
        let before = clock.now();
        clock.sleep(Duration::from_millis(2)).await;
        assert!(clock.now() > before);
    }
}
