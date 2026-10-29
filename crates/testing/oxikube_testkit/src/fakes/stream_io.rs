//! Byte-stream fakes: [`FakePortForwardPort`] (the exec stream fake is in `exec_stream`).
//!
//! Each scripted connection comes with a capture handle ([`ForwardCapture`]) the test keeps to
//! inspect the bytes the app forwarded and to close it.

use std::io::{self, Cursor, Read};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures::FutureExt;
use futures::channel::oneshot;
use futures::io::{AsyncRead, AsyncWrite};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    ForwardHandle, PodRef, PortForwardConnection, PortForwardError, PortForwardPort,
};
use parking_lot::Mutex;

use super::forward_start::ForwardStarts;
use crate::script::{CallLog, Script};

// --- PortForwardPort ---------------------------------------------------------------------

#[derive(Default)]
struct ForwardShared {
    written: Vec<u8>,
    closed: Option<oneshot::Sender<Option<OxiError>>>,
}

/// The test's view of one scripted port-forward connection: the bytes the app sent and a
/// way to close it.
#[derive(Clone)]
pub struct ForwardCapture {
    shared: Arc<Mutex<ForwardShared>>,
}

impl std::fmt::Debug for ForwardCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardCapture")
            .field("written_bytes", &self.shared.lock().written.len())
            .finish()
    }
}

impl ForwardCapture {
    /// Every byte the app wrote to the connection, concatenated.
    pub fn written(&self) -> Vec<u8> {
        self.shared.lock().written.clone()
    }

    /// Resolves the connection's `closed` future with `error` (`None` for a clean close).
    /// Returns `false` when it was already closed.
    pub fn close(&self, error: Option<OxiError>) -> bool {
        match self.shared.lock().closed.take() {
            Some(tx) => tx.send(error).is_ok(),
            None => false,
        }
    }
}

/// What one scripted port-forward connection does: the bytes the remote port answers
/// with. The connection's `closed` future stays pending until
/// [`ForwardCapture::close`], or until the connection's stream and every capture handle
/// are dropped (a clean close).
#[derive(Default)]
pub struct ForwardScript {
    response: Vec<u8>,
    shared: Arc<Mutex<ForwardShared>>,
}

impl std::fmt::Debug for ForwardScript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardScript")
            .field("response_bytes", &self.response.len())
            .finish()
    }
}

impl ForwardScript {
    /// A connection whose remote side answers with `response` and then EOF.
    pub fn new(response: impl Into<Vec<u8>>) -> Self {
        Self {
            response: response.into(),
            shared: Arc::default(),
        }
    }

    /// The handle the test keeps to inspect written bytes and close the connection.
    pub fn capture(&self) -> ForwardCapture {
        ForwardCapture {
            shared: self.shared.clone(),
        }
    }

    fn into_connection(self) -> PortForwardConnection {
        let (tx, rx) = oneshot::channel();
        self.shared.lock().closed = Some(tx);
        PortForwardConnection {
            stream: Box::pin(ScriptedDuplex {
                read: Cursor::new(self.response),
                shared: self.shared,
            }),
            closed: rx.map(Result::unwrap_or_default).boxed(),
        }
    }
}

/// Reads from a fixed buffer, appends writes to the shared capture.
struct ScriptedDuplex {
    read: Cursor<Vec<u8>>,
    shared: Arc<Mutex<ForwardShared>>,
}

impl AsyncRead for ScriptedDuplex {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(self.read.read(buf))
    }
}

impl AsyncWrite for ScriptedDuplex {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.shared.lock().written.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Queued connections for [`FakePortForwardPort`].
#[derive(Debug, Default)]
pub struct PortForwardScripts {
    /// `forward`.
    pub forward: Script<ForwardScript>,
}

/// One call made on a [`FakePortForwardPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortForwardCall {
    /// `forward`.
    Forward {
        /// Pod namespace.
        namespace: String,
        /// Pod name.
        pod: String,
        /// Pod port.
        port: u16,
    },
    /// `start`.
    Start {
        /// The pod.
        target: PodRef,
        /// Pod port.
        remote_port: u16,
        /// Requested bind address.
        bind: std::net::IpAddr,
        /// Requested local port (`None`: any free one).
        local_port: Option<u16>,
    },
}

/// Fake `PortForwardPort`.
///
/// * `forward`: scripted; each call takes the next [`ForwardScript`].
/// * `start`: a real loopback listener that echoes every byte back and counts it in the
///   handle's stats. [`fail_next_start`](Self::fail_next_start) queues start failures;
///   [`inject_error`](Self::inject_error) reports a runtime error on every live forward
///   (a pod going away, a lost connection). Needs a tokio runtime.
#[derive(Debug, Default)]
pub struct FakePortForwardPort {
    script: PortForwardScripts,
    calls: CallLog<PortForwardCall>,
    starts: ForwardStarts,
}

fake_plumbing!(FakePortForwardPort, PortForwardScripts, PortForwardCall);

impl FakePortForwardPort {
    /// A fake with nothing scripted.
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes the next `start` fail with `err` (queued in call order).
    pub fn fail_next_start(&self, err: PortForwardError) {
        self.starts.fail_next(err);
    }

    /// Reports `err` on the error stream of every live forward; returns how many got it.
    pub fn inject_error(&self, err: &PortForwardError) -> usize {
        self.starts.inject(err)
    }
}

#[async_trait]
impl PortForwardPort for FakePortForwardPort {
    async fn forward(
        &self,
        namespace: &str,
        pod: &str,
        port: u16,
    ) -> OxiResult<PortForwardConnection> {
        self.calls.record(PortForwardCall::Forward {
            namespace: namespace.to_owned(),
            pod: pod.to_owned(),
            port,
        });
        let script = self
            .script
            .forward
            .next_or_unscripted("FakePortForwardPort", "forward")?;
        Ok(script.into_connection())
    }

    async fn start(
        &self,
        target: PodRef,
        remote_port: u16,
        bind: std::net::IpAddr,
        local_port: Option<u16>,
    ) -> Result<ForwardHandle, PortForwardError> {
        self.calls.record(PortForwardCall::Start {
            target: target.clone(),
            remote_port,
            bind,
            local_port,
        });
        self.starts
            .start(target, remote_port, bind, local_port)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use futures::io::{AsyncReadExt, AsyncWriteExt};
    use oxikube_domain::ErrorKind;

    #[test]
    fn port_forward_reads_response_records_writes_and_closes() {
        let fake = FakePortForwardPort::new();
        let script = ForwardScript::new("HTTP/1.1 200 OK\r\n\r\n");
        let capture = script.capture();
        fake.script()
            .forward
            .push_ok(script)
            .push_err(OxiError::not_found("no pod"));
        let mut conn = block_on(fake.forward("demo", "web", 8080)).unwrap();
        block_on(conn.stream.write_all(b"GET / HTTP/1.1\r\n\r\n")).unwrap();
        let mut response = String::new();
        block_on(conn.stream.read_to_string(&mut response)).unwrap();
        assert_eq!(response, "HTTP/1.1 200 OK\r\n\r\n");
        assert_eq!(capture.written(), b"GET / HTTP/1.1\r\n\r\n");
        let mut closed = conn.closed;
        assert!((&mut closed).now_or_never().is_none());
        assert!(capture.close(Some(OxiError::network("pod restarted"))));
        assert!(block_on(closed).is_some_and(|e| e.is_retryable()));

        assert_eq!(
            block_on(fake.forward("demo", "web", 8080))
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            fake.recorded_calls(),
            vec![
                PortForwardCall::Forward {
                    namespace: "demo".into(),
                    pod: "web".into(),
                    port: 8080
                };
                2
            ]
        );
    }
}
