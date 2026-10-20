//! The forward's task: the accept loop, the connection cap and the graceful stop.

use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use oxikube_domain::{ErrorKind, OxiError};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::bridge::Bridge;
use super::hub::StopGuard;
use super::monitor::{Flow, Monitor};
use super::plan::PodInfo;

/// Pause after a failed `accept` (out of file descriptors, ...) so a persistent failure
/// cannot spin the task.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// How long a graceful stop lets open connections finish before they are aborted.
pub(super) const STOP_GRACE: Duration = Duration::from_secs(2);

/// Concurrent connections one forward serves by default. Each opens its own websocket to
/// the API server, so the cap protects the server (and the local file-descriptor table)
/// from a runaway client.
pub const DEFAULT_MAX_CONNECTIONS: usize = 256;

/// The listener, the pod watch and everything the loop needs.
pub(super) struct Accept {
    pub(super) listener: TcpListener,
    pub(super) pods: BoxStream<'static, Vec<PodInfo>>,
    pub(super) monitor: Monitor,
    pub(super) bridge: Bridge,
    pub(super) stop: StopGuard,
    pub(super) max_connections: usize,
}

impl Accept {
    /// Runs the forward. Owns the listener and every bridge: when it ends or is aborted they
    /// all go with it.
    ///
    /// Ends when the shutdown token is cancelled, when a pod forward loses its pod, or when
    /// the pod watch ends. On the way out it stops accepting first, asks every open
    /// connection to close cleanly, waits up to [`STOP_GRACE`], then aborts the rest.
    pub(super) async fn run(self) {
        let Accept {
            listener,
            mut pods,
            mut monitor,
            bridge,
            stop,
            max_connections,
        } = self;
        // Cancelled by a graceful stop; the bridges watch the same token.
        let shutdown = bridge.shutdown.clone();
        let mut bridges = JoinSet::new();
        loop {
            tokio::select! {
                () = shutdown.cancelled() => break,
                accepted = listener.accept() => match accepted {
                    Ok((local, _peer)) => {
                        // Reap first so finished connections do not count against the cap.
                        while bridges.try_join_next().is_some() {}
                        if bridges.len() >= max_connections {
                            // Refuse by closing at once; the client sees the connection end.
                            drop(local);
                            stop.0.error(&too_many(max_connections));
                            continue;
                        }
                        bridges.spawn(bridge.clone().serve(local));
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "port-forward accept failed");
                        stop.0.error(&OxiError::internal(format!(
                            "accepting a local connection failed: {err}"
                        )));
                        tokio::time::sleep(ACCEPT_BACKOFF).await;
                    }
                },
                snapshot = pods.next() => match snapshot {
                    Some(pods) => {
                        if monitor.on_snapshot(&pods) == Flow::Stop {
                            break;
                        }
                    }
                    None => {
                        stop.0.error(&OxiError::internal("the pod watch ended unexpectedly"));
                        break;
                    }
                },
                // Reap finished bridges so the set does not grow with every connection.
                Some(_) = bridges.join_next(), if !bridges.is_empty() => {}
            }
        }
        // Stop accepting, then let open connections close cleanly within the grace period.
        drop(listener);
        shutdown.cancel();
        let drained = tokio::time::timeout(STOP_GRACE, async {
            while bridges.join_next().await.is_some() {}
        })
        .await;
        if drained.is_err() {
            tracing::debug!(
                left = bridges.len(),
                "port-forward connections aborted after the grace period"
            );
        }
        bridges.shutdown().await;
        // `stop` publishes `Stopped` as it drops, after the listener and bridges are gone.
        drop(stop);
    }
}

/// The error a refused connection publishes. Constant wording, so a client hammering the
/// forward produces one status transition, not one per connection.
fn too_many(max: usize) -> OxiError {
    OxiError::new(
        ErrorKind::BudgetExceeded,
        format!(
            "connection refused: this forward already serves {max} connections \
             (close some, or raise the per-forward limit)"
        ),
    )
}
