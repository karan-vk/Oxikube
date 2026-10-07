//! Keeping a live shell's pod stamped alive, so the leftover sweep can tell it from an orphan.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use oxikube_domain::{OxiError, OxiResult};
use tokio::runtime::Handle;
use tokio::task::AbortHandle;
use tokio::time::{Instant, MissedTickBehavior};

use super::super::pods::Pods;

/// How often an open shell stamps its pod. The app's leftover grace is many times this, so a
/// few missed stamps (a slow API server, a laptop that napped) never make a live shell look
/// abandoned.
pub(super) const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

/// Now, in seconds since the epoch.
pub(super) fn now_secs() -> OxiResult<i64> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OxiError::internal("the system clock is before 1970"))?
        .as_secs();
    Ok(i64::try_from(secs).unwrap_or(i64::MAX))
}

/// The task that stamps one pod; ends when dropped.
pub(super) struct Heartbeat(AbortHandle);

impl Heartbeat {
    /// Starts stamping `namespace/name` every [`HEARTBEAT_INTERVAL`] on `runtime`.
    pub(super) fn start(
        runtime: &Handle,
        pods: Arc<dyn Pods>,
        namespace: String,
        name: String,
    ) -> Self {
        let task = runtime.spawn(async move {
            let mut tick =
                tokio::time::interval_at(Instant::now() + HEARTBEAT_INTERVAL, HEARTBEAT_INTERVAL);
            tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
            let mut failing = false;
            loop {
                tick.tick().await;
                let stamped = match now_secs() {
                    Ok(now) => pods.heartbeat(&namespace, &name, now).await,
                    Err(err) => Err(err),
                };
                match stamped {
                    Ok(()) => failing = false,
                    // Said once per streak: the cause is usually RBAC (no `patch` on pods), and
                    // it then repeats every minute for the whole session.
                    Err(err) if !failing => {
                        failing = true;
                        tracing::warn!(
                            namespace = %namespace, pod = %name, error = %err,
                            "could not stamp the node shell pod alive; another Oxikube's \
                             leftover sweep may delete it once it is old enough"
                        );
                    }
                    Err(_) => {}
                }
            }
        });
        Self(task.abort_handle())
    }
}

impl Drop for Heartbeat {
    fn drop(&mut self) {
        self.0.abort();
    }
}
