//! Port forwarding to pods: [`PortForwardPort`].
//!
//! Mirrors kube's `Api<Pod>::portforward(name, &[port])`, whose `Portforwarder`
//! hands out one `AsyncRead + AsyncWrite` stream per port (`take_stream`) and a
//! future that resolves with the port's error (`take_error`). Here that is one
//! [`PortForwardConnection`] per call, using the runtime-neutral
//! `futures::io` traits; the adapter bridges tokio's with `tokio_util::compat`.
//! Local TCP listeners, service-to-pod resolution and restarts belong to the
//! app's `PortForwardManager`, not to this port.

use std::pin::Pin;

use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::io::{AsyncRead, AsyncWrite};
use oxikube_domain::{OxiError, OxiResult};

/// A bidirectional byte stream: [`AsyncRead`] + [`AsyncWrite`], `Send`.
///
/// Implemented for every type with those bounds, so it can be used as
/// `dyn DuplexStream`.
pub trait DuplexStream: AsyncRead + AsyncWrite + Send {}

impl<T: AsyncRead + AsyncWrite + Send + ?Sized> DuplexStream for T {}

/// One forwarded connection to a pod port.
///
/// Dropping it closes the connection.
pub struct PortForwardConnection {
    /// The byte stream to and from the pod port.
    pub stream: Pin<Box<dyn DuplexStream>>,
    /// Resolves when the forward ends: `Some(error)` if the server reported
    /// one, `None` on a clean close. The stream is unusable afterwards.
    pub closed: BoxFuture<'static, Option<OxiError>>,
}

impl std::fmt::Debug for PortForwardConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PortForwardConnection")
            .finish_non_exhaustive()
    }
}

/// Opens port-forward connections to pods.
///
/// Not a `MutationGuard` operation; callers gate it on the session's
/// port-forward capability.
#[async_trait]
pub trait PortForwardPort: Send + Sync {
    /// Opens a connection to `port` on `pod` in `namespace`.
    async fn forward(
        &self,
        namespace: &str,
        pod: &str,
        port: u16,
    ) -> OxiResult<PortForwardConnection>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use futures::io::{AsyncReadExt, AsyncWriteExt, Cursor};

    #[test]
    fn connection_streams_bytes_both_ways() {
        let mut conn = PortForwardConnection {
            stream: Box::pin(Cursor::new(b"HTTP/1.1 200 OK".to_vec())),
            closed: Box::pin(async { None }),
        };
        assert!(format!("{conn:?}").starts_with("PortForwardConnection"));
        block_on(async {
            let mut buf = [0u8; 8];
            conn.stream.read_exact(&mut buf).await.expect("read");
            assert_eq!(&buf, b"HTTP/1.1");
            conn.stream.write_all(b"!").await.expect("write");
            assert!(conn.closed.await.is_none());
        });
    }
}
