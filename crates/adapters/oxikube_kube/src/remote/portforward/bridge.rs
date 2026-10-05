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
use oxikube_ports::{PortForwardConnection, PortForwardPort};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, copy_bidirectional};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_util::compat::FuturesAsyncReadCompatExt;

use super::hub::StatusHub;
use super::plan::Target;

/// How long, after the byte stream ended, to wait for the pod's error message. The server
/// sends the error before it closes the data channel, but the two reach us through different
/// tasks.
const ERROR_GRACE: Duration = Duration::from_millis(150);

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
}

impl Bridge {
    /// Serves `local` until either side closes. Failures are published, never returned.
    pub(super) async fn serve(self, mut local: TcpStream) {
        let Some(target) = self.target.borrow().clone() else {
            // Between pods: refuse by closing, the client sees a reset.
            tracing::debug!("port-forward connection refused: no target pod right now");
            return;
        };
        // Interactive protocols (ssh, redis) must not wait for Nagle.
        let _ = local.set_nodelay(true);
        let connection = match self
            .connector
            .forward(&self.namespace, &target.pod, target.port)
            .await
        {
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
        };
        let mut closed: BoxFuture<'static, Option<OxiError>> = closed;

        let copy = copy_bidirectional(&mut local, &mut remote);
        tokio::pin!(copy);
        let mut closed_early = false;
        let result = loop {
            tokio::select! {
                result = &mut copy => break result,
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
