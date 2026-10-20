//! One accepted local connection bridged to one pod connection.
//!
//! Every connection opens its own `portforward` websocket (what kubectl does): a failure or a
//! slow client on one never touches another.

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use futures::future::BoxFuture;
use oxikube_domain::OxiError;
use oxikube_ports::{ForwardStats, PortForwardConnection, PortForwardPort};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf, copy_bidirectional};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tokio_util::sync::CancellationToken;

use super::accept::STOP_GRACE;
use super::hub::StatusHub;
use super::plan::Target;

/// How long, after the byte stream ended, to wait for the pod's error message. The server
/// sends the error before it closes the data channel, but the two reach us through different
/// tasks.
const ERROR_GRACE: Duration = Duration::from_millis(150);

/// How long a graceful stop lets one connection drain before it is cut. Shorter than
/// [`STOP_GRACE`] so the bridge can still send its FINs before the accept task aborts it.
const DRAIN: Duration = STOP_GRACE.saturating_sub(Duration::from_millis(250));

/// Marks the forward healthy once, at the first sign that a connection works: a byte from the
/// pod, or a close the pod did not report an error for. Opening the websocket proves nothing,
/// because the pod reports an unreachable port afterwards, on the error channel.
#[derive(Clone)]
struct Recovery {
    hub: StatusHub,
    local_addr: SocketAddr,
    pod: Arc<str>,
    done: Arc<AtomicBool>,
}

impl Recovery {
    fn healthy(&self) {
        if !self.done.swap(true, Ordering::Relaxed) {
            self.hub.recover(self.local_addr, &self.pod);
        }
    }
}

/// The pod side of a bridge; the first non-empty read from it is evidence of a working
/// connection. Everything else passes straight through.
struct Probed<S> {
    inner: S,
    recovery: Recovery,
    stats: Arc<ForwardStats>,
}

impl<S: AsyncRead + Unpin> AsyncRead for Probed<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(polled, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            self.stats
                .add_bytes_in((buf.filled().len() - before) as u64);
            self.recovery.healthy();
        }
        polled
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Probed<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let polled = Pin::new(&mut self.inner).poll_write(cx, data);
        if let Poll::Ready(Ok(written)) = &polled {
            self.stats.add_bytes_out(*written as u64);
        }
        polled
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// The local side of a bridge. Once `stop` is cancelled, a read that would wait reports end of
/// stream instead: what the client already sent still goes to the pod, then the pod side is
/// half-closed, so the pod can finish its reply and hang up while the bridge drains.
struct Drained<S> {
    inner: S,
    stop: CancellationToken,
}

impl<S: AsyncRead + Unpin> AsyncRead for Drained<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        if polled.is_pending() && self.stop.is_cancelled() {
            return Poll::Ready(Ok(()));
        }
        polled
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Drained<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, data)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// What every bridge of one forward shares. Cheap to clone.
#[derive(Clone)]
pub(super) struct Bridge {
    pub(super) connector: Arc<dyn PortForwardPort>,
    pub(super) namespace: Arc<str>,
    pub(super) target: watch::Receiver<Option<Target>>,
    pub(super) hub: StatusHub,
    pub(super) local_addr: SocketAddr,
    pub(super) stats: Arc<ForwardStats>,
    /// Cancelled by a graceful stop: close cleanly and return.
    pub(super) shutdown: CancellationToken,
}

impl Bridge {
    /// Serves `local` until either side closes. Failures are published, never returned.
    pub(super) async fn serve(self, local: TcpStream) {
        let Some(target) = self.target.borrow().clone() else {
            // Between pods: refuse by closing, the client sees a reset.
            tracing::debug!("port-forward connection refused: no target pod right now");
            return;
        };
        let _active = self.stats.open_connection();
        // Interactive protocols (ssh, redis) must not wait for Nagle.
        let _ = local.set_nodelay(true);
        let mut local = Drained {
            inner: local,
            stop: self.shutdown.clone(),
        };
        let opened = tokio::select! {
            () = self.shutdown.cancelled() => return,
            opened = self.connector.forward(&self.namespace, &target.pod, target.port) => opened,
        };
        let connection = match opened {
            Ok(connection) => connection,
            Err(err) => {
                tracing::debug!(kind = %err.kind(), "port-forward connection failed to open");
                self.hub.error(&err);
                return;
            }
        };
        let PortForwardConnection { stream, closed } = connection;
        let recovery = Recovery {
            hub: self.hub.clone(),
            local_addr: self.local_addr,
            pod: target.pod.into(),
            done: Arc::default(),
        };
        let mut remote = Probed {
            inner: stream.compat(),
            recovery: recovery.clone(),
            stats: self.stats.clone(),
        };
        let mut closed: BoxFuture<'static, Option<OxiError>> = closed;

        // Boxed so a graceful stop can drain it, then drop it and use both sockets again.
        let mut copy = Box::pin(copy_bidirectional(&mut local, &mut remote));
        let mut closed_early = false;
        let result = loop {
            tokio::select! {
                result = &mut copy => break result,
                () = self.shutdown.cancelled() => {
                    // Graceful stop: keep copying so in-flight bytes reach both ends. `Drained`
                    // now ends the client's side once it has nothing more to read, which
                    // half-closes the pod side; the copy finishes when the pod hangs up.
                    if tokio::time::timeout(DRAIN, &mut copy).await.is_err() {
                        tracing::debug!("port-forward connection cut after the drain window");
                    }
                    // Send FIN both ways, so the client sees a clean end of stream.
                    drop(copy);
                    let _ = local.shutdown().await;
                    let _ = remote.shutdown().await;
                    return;
                }
                // A server-reported error ends the forward at once. A clean close (`None`)
                // does not: bytes may still be queued for the local side.
                reported = &mut closed, if !closed_early => {
                    closed_early = true;
                    match reported {
                        Some(err) => {
                            self.hub.error(&err);
                            return;
                        }
                        None => recovery.healthy(),
                    }
                }
            }
        };
        if let Err(err) = &result {
            tracing::debug!(error = %err, "port-forward copy ended with an I/O error");
        }
        if !closed_early {
            // The stream ended first; the error that explains why may still be on its way.
            match tokio::time::timeout(ERROR_GRACE, closed).await {
                Ok(Some(err)) => self.hub.error(&err),
                Ok(None) => recovery.healthy(),
                // Nothing was reported in time: not evidence either way.
                Err(_) => {}
            }
        }
    }
}
