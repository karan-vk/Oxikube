//! [`ForwardHandle`]: the owner of a running forward.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::ForwardStatus;
use oxikube_ports::ForwardStats;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::accept::STOP_GRACE;
use super::hub::StatusHub;

/// Slack on top of the grace period before a graceful stop gives up waiting.
const STOP_SLACK: Duration = Duration::from_millis(500);

/// A running forward: a local listener bridged to a pod.
///
/// **Dropping the handle stops the forward at once**: the listener closes (the port is free
/// again) and every bridged connection is torn down. [`stop`](Self::stop) is the graceful
/// spelling: it stops accepting, lets open connections close cleanly within a 2 s grace
/// period, and waits for all of it.
///
/// Status is published two ways. [`status`](Self::status) and [`watch_status`](Self::watch_status)
/// give the latest value; [`events`](Self::events) delivers every transition, so a
/// `TargetGone` that recovers before anyone looks is still seen. The last status is always
/// `Stopped`.
#[derive(Debug)]
pub struct ForwardHandle {
    local_addr: SocketAddr,
    hub: StatusHub,
    stats: Arc<ForwardStats>,
    shutdown: CancellationToken,
    task: JoinHandle<()>,
}

impl ForwardHandle {
    pub(super) fn new(
        local_addr: SocketAddr,
        hub: StatusHub,
        stats: Arc<ForwardStats>,
        shutdown: CancellationToken,
        task: JoinHandle<()>,
    ) -> Self {
        Self {
            local_addr,
            hub,
            stats,
            shutdown,
            task,
        }
    }

    /// Live traffic counters (atomics; sample them on a timer).
    pub fn stats(&self) -> &Arc<ForwardStats> {
        &self.stats
    }

    /// The bound local address, with the real port when the spec asked for port `0`.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The latest status.
    pub fn status(&self) -> ForwardStatus {
        self.hub.latest()
    }

    /// A receiver that always holds the latest status.
    pub fn watch_status(&self) -> watch::Receiver<ForwardStatus> {
        self.hub.watch()
    }

    /// Every status transition from now on. A subscriber that falls more than 64 behind gets
    /// `Lagged` and resumes at the oldest transition kept.
    pub fn events(&self) -> broadcast::Receiver<ForwardStatus> {
        self.hub.subscribe()
    }

    /// Whether the forward has ended (for a pod forward whose pod went away, or after
    /// [`stop`](Self::stop)).
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Stops the forward gracefully and waits until it has ended: the listener closes first,
    /// open connections are asked to close cleanly and get up to 2 s, then whatever is left is
    /// aborted. The last status is `Stopped`.
    pub async fn stop(mut self) {
        self.shutdown.cancel();
        if tokio::time::timeout(STOP_GRACE + STOP_SLACK, &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
            // `Err(Cancelled)` is the expected outcome of the abort.
            let _ = (&mut self.task).await;
        }
    }

    /// Resolves once the forward has ended, whoever ended it. Does not stop it.
    pub fn stopped(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut status = self.hub.watch();
        async move {
            // A closed channel means the hub is gone, so the forward is too.
            let _ = status.wait_for(ForwardStatus::is_terminal).await;
        }
    }

    /// A graceful stop that does not borrow the handle: cancels the forward and resolves once
    /// it has ended or the grace period (plus slack) ran out. The handle's drop still aborts.
    pub(super) fn graceful_stop(&self) -> impl Future<Output = ()> + Send + 'static {
        let shutdown = self.shutdown.clone();
        let stopped = self.stopped();
        async move {
            shutdown.cancel();
            let _ = tokio::time::timeout(STOP_GRACE + STOP_SLACK, stopped).await;
        }
    }
}

impl Drop for ForwardHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}
