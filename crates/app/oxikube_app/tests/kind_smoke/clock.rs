//! A real-time `ClockPort`: the production one lives in `oxikube_runtime`, which the app crate
//! does not depend on. The smoke test runs on the wall clock because the cluster is real.

use std::time::Duration;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_ports::ClockPort;

/// `now()` is the system clock and `sleep` is Tokio's timer.
pub struct TokioClock;

#[async_trait]
impl ClockPort for TokioClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}
