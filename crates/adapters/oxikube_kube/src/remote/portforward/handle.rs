//! [`ForwardHandle`]: the owner of a running forward.

use std::net::SocketAddr;

use oxikube_domain::ForwardStatus;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

use super::hub::StatusHub;

/// A running forward: a local listener bridged to a pod.
///
/// **Dropping the handle stops the forward**: the listener closes (the port is free again)
/// and every bridged connection is torn down. [`stop`](Self::stop) does the same and waits
/// for it.
///
/// Status is published two ways. [`status`](Self::status) and [`watch_status`](Self::watch_status)
/// give the latest value; [`events`](Self::events) delivers every transition, so a
/// `TargetGone` that recovers before anyone looks is still seen. The last status is always
/// `Stopped`.
#[derive(Debug)]
pub struct ForwardHandle {
    local_addr: SocketAddr,
    hub: StatusHub,
    task: JoinHandle<()>,
}

impl ForwardHandle {
    pub(super) fn new(local_addr: SocketAddr, hub: StatusHub, task: JoinHandle<()>) -> Self {
        Self {
            local_addr,
            hub,
            task,
        }
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

    /// Stops the forward and waits until the listener is closed.
    pub async fn stop(mut self) {
        self.task.abort();
        // `Err(Cancelled)` is the expected outcome of the abort.
        let _ = (&mut self.task).await;
    }
}

impl Drop for ForwardHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}
