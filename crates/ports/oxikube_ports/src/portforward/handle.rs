//! [`ForwardHandle`], [`ForwardStats`] and [`PodRef`]: what [`PortForwardPort::start`]
//! hands back and takes.
//!
//! [`PortForwardPort::start`]: super::PortForwardPort::start

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::channel::mpsc;

use super::PortForwardError;

/// A pod in the cluster the adapter serves.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PodRef {
    /// Pod namespace.
    pub namespace: String,
    /// Pod name.
    pub name: String,
}

impl PodRef {
    /// `namespace/name`.
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
        }
    }
}

impl std::fmt::Display for PodRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.namespace, self.name)
    }
}

/// Traffic counters of one forward. Lock-free: the data path does relaxed atomic
/// increments (never an event per read) and the UI samples [`snapshot`](Self::snapshot) on a
/// timer.
#[derive(Debug, Default)]
pub struct ForwardStats {
    bytes_from_pod: AtomicU64,
    bytes_to_pod: AtomicU64,
    active: AtomicU64,
    total: AtomicU64,
}

/// A point-in-time copy of [`ForwardStats`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ForwardStatsSnapshot {
    /// Bytes received from the pod (downloads).
    pub bytes_in: u64,
    /// Bytes sent to the pod (uploads).
    pub bytes_out: u64,
    /// Connections open right now.
    pub active_connections: u64,
    /// Connections accepted since the forward started.
    pub total_connections: u64,
}

impl ForwardStats {
    /// Counts `n` bytes that came from the pod.
    pub fn add_bytes_in(&self, n: u64) {
        self.bytes_from_pod.fetch_add(n, Ordering::Relaxed);
    }

    /// Counts `n` bytes sent to the pod.
    pub fn add_bytes_out(&self, n: u64) {
        self.bytes_to_pod.fetch_add(n, Ordering::Relaxed);
    }

    /// Counts a new connection; it is "active" until the guard drops.
    #[must_use = "the connection counts as active until the guard is dropped"]
    pub fn open_connection(self: &Arc<Self>) -> ActiveConnection {
        self.active.fetch_add(1, Ordering::Relaxed);
        self.total.fetch_add(1, Ordering::Relaxed);
        ActiveConnection(Arc::clone(self))
    }

    /// Reads every counter.
    pub fn snapshot(&self) -> ForwardStatsSnapshot {
        ForwardStatsSnapshot {
            bytes_in: self.bytes_from_pod.load(Ordering::Relaxed),
            bytes_out: self.bytes_to_pod.load(Ordering::Relaxed),
            active_connections: self.active.load(Ordering::Relaxed),
            total_connections: self.total.load(Ordering::Relaxed),
        }
    }
}

/// Marks one connection active; see [`ForwardStats::open_connection`].
#[derive(Debug)]
pub struct ActiveConnection(Arc<ForwardStats>);

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

/// A running forward: a local listener bridged to a pod.
///
/// **Dropping the handle stops the forward** and frees the port; [`stop`](Self::stop) is the
/// explicit spelling.
pub struct ForwardHandle {
    /// The bound address, with the real port when `0` was requested.
    pub local_addr: SocketAddr,
    /// Live traffic counters.
    pub stats: Arc<ForwardStats>,
    /// Failures after the start: a connection that failed, the pod going away, the forward
    /// ending. The stream ends when the forward does.
    pub errors: mpsc::UnboundedReceiver<PortForwardError>,
    // Dropping the owner stops the forward. Type-erased so the port stays runtime-neutral.
    owner: Box<dyn Send>,
}

impl ForwardHandle {
    /// A handle whose `owner` stops the forward when dropped (it should abort the tasks that
    /// serve it).
    pub fn new(
        local_addr: SocketAddr,
        stats: Arc<ForwardStats>,
        errors: mpsc::UnboundedReceiver<PortForwardError>,
        owner: impl Send + 'static,
    ) -> Self {
        Self {
            local_addr,
            stats,
            errors,
            owner: Box::new(owner),
        }
    }

    /// Stops the forward: the listener closes and every connection is torn down.
    pub fn stop(self) {
        drop(self.owner);
    }
}

impl std::fmt::Debug for ForwardHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardHandle")
            .field("local_addr", &self.local_addr)
            .field("stats", &self.stats.snapshot())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_count_without_locks_and_guard_decrements() {
        let stats = Arc::new(ForwardStats::default());
        let a = stats.open_connection();
        let _b = stats.open_connection();
        stats.add_bytes_in(10);
        stats.add_bytes_out(3);
        drop(a);
        assert_eq!(
            stats.snapshot(),
            ForwardStatsSnapshot {
                bytes_in: 10,
                bytes_out: 3,
                active_connections: 1,
                total_connections: 2
            }
        );
    }

    #[test]
    fn stop_and_drop_release_the_owner() {
        struct Flag(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Flag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_tx, rx) = mpsc::unbounded();
        let handle = ForwardHandle::new(
            SocketAddr::from(([127, 0, 0, 1], 1)),
            Arc::default(),
            rx,
            Flag(flag.clone()),
        );
        assert!(format!("{handle:?}").contains("127.0.0.1:1"));
        handle.stop();
        assert!(flag.load(Ordering::SeqCst));
    }
}
