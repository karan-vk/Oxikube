//! Byte-stream fakes: [`FakeExecPort`] and [`FakePortForwardPort`].
//!
//! Each scripted session comes with a capture handle ([`ExecCapture`],
//! [`ForwardCapture`]) the test keeps to inspect what the app wrote (stdin, resizes,
//! forwarded bytes) and to end the session.

use std::io::{self, Cursor, Read};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures::channel::oneshot;
use futures::io::{AsyncRead, AsyncWrite};
use futures::{FutureExt, StreamExt, sink, stream};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    ExecOptions, ExecPort, ExecSession, ExitStatus, PortForwardConnection, PortForwardPort,
    TerminalSize,
};
use parking_lot::Mutex;

use crate::script::{CallLog, Script};

// --- ExecPort ----------------------------------------------------------------------------

#[derive(Default)]
struct ExecShared {
    stdin: Vec<u8>,
    resizes: Vec<TerminalSize>,
    exit: Option<oneshot::Sender<OxiResult<ExitStatus>>>,
}

/// The test's view of one scripted exec session: what the app wrote to stdin, the
/// terminal sizes it sent, and a way to end the session.
#[derive(Clone)]
pub struct ExecCapture {
    shared: Arc<Mutex<ExecShared>>,
}

impl std::fmt::Debug for ExecCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let shared = self.shared.lock();
        f.debug_struct("ExecCapture")
            .field("stdin_bytes", &shared.stdin.len())
            .field("resizes", &shared.resizes)
            .finish()
    }
}

impl ExecCapture {
    /// Every byte the app wrote to stdin, concatenated.
    pub fn stdin(&self) -> Vec<u8> {
        self.shared.lock().stdin.clone()
    }

    /// Every terminal size the app sent, in order.
    pub fn resizes(&self) -> Vec<TerminalSize> {
        self.shared.lock().resizes.clone()
    }

    /// Ends a session scripted with [`ExecScript::exit_when_told`]: its `status`
    /// future resolves to `status`. Returns `false` when the session already ended.
    pub fn finish(&self, status: OxiResult<ExitStatus>) -> bool {
        match self.shared.lock().exit.take() {
            Some(tx) => tx.send(status).is_ok(),
            None => false,
        }
    }
}

/// What one scripted exec or attach session does: the output it produces and how it
/// ends.
///
/// By default the session prints nothing and exits with code 0 as soon as the app awaits
/// `status`. Output is delivered immediately, chunk by chunk, in order.
pub struct ExecScript {
    stdout: Vec<Vec<u8>>,
    stderr: Vec<Vec<u8>>,
    status: Option<OxiResult<ExitStatus>>,
    shared: Arc<Mutex<ExecShared>>,
}

impl Default for ExecScript {
    fn default() -> Self {
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
            status: Some(Ok(ExitStatus::success())),
            shared: Arc::default(),
        }
    }
}

impl std::fmt::Debug for ExecScript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecScript")
            .field("stdout_chunks", &self.stdout.len())
            .field("stderr_chunks", &self.stderr.len())
            .finish_non_exhaustive()
    }
}

impl ExecScript {
    /// A session with no output that exits 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one stdout chunk.
    #[must_use]
    pub fn stdout(mut self, chunk: impl Into<Vec<u8>>) -> Self {
        self.stdout.push(chunk.into());
        self
    }

    /// Adds one stderr chunk.
    #[must_use]
    pub fn stderr(mut self, chunk: impl Into<Vec<u8>>) -> Self {
        self.stderr.push(chunk.into());
        self
    }

    /// The session ends with `status` (an exit code or an error).
    #[must_use]
    pub fn exit(mut self, status: OxiResult<ExitStatus>) -> Self {
        self.status = Some(status);
        self
    }

    /// The session stays running until the test calls [`ExecCapture::finish`].
    #[must_use]
    pub fn exit_when_told(mut self) -> Self {
        self.status = None;
        self
    }

    /// The handle the test keeps to inspect stdin and resizes.
    pub fn capture(&self) -> ExecCapture {
        ExecCapture {
            shared: self.shared.clone(),
        }
    }

    fn into_session(self, options: &ExecOptions) -> ExecSession {
        let Self {
            stdout,
            stderr,
            status,
            shared,
        } = self;
        let status = match status {
            Some(status) => futures::future::ready(status).boxed(),
            None => {
                let (tx, rx) = oneshot::channel();
                shared.lock().exit = Some(tx);
                rx.map(|r| r.unwrap_or_else(|_| Err(OxiError::network("exec session dropped"))))
                    .boxed()
            }
        };
        let output = |chunks: Vec<Vec<u8>>| stream::iter(chunks.into_iter().map(Ok)).boxed();
        let stdin_shared = shared.clone();
        let stdin = sink::unfold(stdin_shared, |shared, bytes: Vec<u8>| async move {
            shared.lock().stdin.extend_from_slice(&bytes);
            Ok::<_, OxiError>(shared)
        });
        let resize = sink::unfold(shared, |shared, size: TerminalSize| async move {
            shared.lock().resizes.push(size);
            Ok::<_, OxiError>(shared)
        });
        ExecSession {
            stdin: options.stdin.then(|| Box::pin(stdin) as _),
            stdout: options.stdout.then(|| output(stdout)),
            stderr: (options.stderr && !options.tty).then(|| output(stderr)),
            resize: options.tty.then(|| Box::pin(resize) as _),
            status,
        }
    }
}

/// Queued sessions for each [`FakeExecPort`] method.
#[derive(Debug, Default)]
pub struct ExecScripts {
    /// `exec`.
    pub exec: Script<ExecScript>,
    /// `attach`.
    pub attach: Script<ExecScript>,
}

/// One call made on a [`FakeExecPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecCall {
    /// `exec`.
    Exec {
        /// Pod namespace.
        namespace: String,
        /// Pod name.
        pod: String,
        /// Command argv.
        command: Vec<String>,
        /// Options passed.
        options: ExecOptions,
    },
    /// `attach`.
    Attach {
        /// Pod namespace.
        namespace: String,
        /// Pod name.
        pod: String,
        /// Options passed.
        options: ExecOptions,
    },
}

/// Fake `ExecPort`. Scripted only: each call takes the next [`ExecScript`]. The session
/// exposes stdin/stdout/stderr/resize exactly as the options ask (stderr is absent with a
/// TTY, as on a real cluster).
#[derive(Debug, Default)]
pub struct FakeExecPort {
    script: ExecScripts,
    calls: CallLog<ExecCall>,
}

fake_plumbing!(FakeExecPort, ExecScripts, ExecCall);

impl FakeExecPort {
    /// A fake with nothing scripted.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ExecPort for FakeExecPort {
    async fn exec(
        &self,
        namespace: &str,
        pod: &str,
        command: &[String],
        options: &ExecOptions,
    ) -> OxiResult<ExecSession> {
        self.calls.record(ExecCall::Exec {
            namespace: namespace.to_owned(),
            pod: pod.to_owned(),
            command: command.to_vec(),
            options: options.clone(),
        });
        let script = self
            .script
            .exec
            .next_or_unscripted("FakeExecPort", "exec")?;
        Ok(script.into_session(options))
    }

    async fn attach(
        &self,
        namespace: &str,
        pod: &str,
        options: &ExecOptions,
    ) -> OxiResult<ExecSession> {
        self.calls.record(ExecCall::Attach {
            namespace: namespace.to_owned(),
            pod: pod.to_owned(),
            options: options.clone(),
        });
        let script = self
            .script
            .attach
            .next_or_unscripted("FakeExecPort", "attach")?;
        Ok(script.into_session(options))
    }
}

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
}

/// Fake `PortForwardPort`. Scripted only: each call takes the next [`ForwardScript`].
#[derive(Debug, Default)]
pub struct FakePortForwardPort {
    script: PortForwardScripts,
    calls: CallLog<PortForwardCall>,
}

fake_plumbing!(FakePortForwardPort, PortForwardScripts, PortForwardCall);

impl FakePortForwardPort {
    /// A fake with nothing scripted.
    pub fn new() -> Self {
        Self::default()
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
}

/// Sends every item of `items` into `sink`, then closes it. Shared by tests.
#[cfg(test)]
async fn send_all<S, T>(sink: &mut S, items: Vec<T>) -> Result<(), S::Error>
where
    S: futures::Sink<T> + Unpin,
{
    use futures::SinkExt;
    for item in items {
        sink.send(item).await?;
    }
    sink.close().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use futures::io::{AsyncReadExt, AsyncWriteExt};
    use oxikube_domain::ErrorKind;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn exec_scripted_session_produces_output_and_captures_stdin() {
        let fake = FakeExecPort::new();
        let script = ExecScript::new()
            .stdout("hello ")
            .stdout("world")
            .stderr("warn")
            .exit(Ok(ExitStatus {
                code: Some(2),
                message: None,
            }));
        let capture = script.capture();
        fake.script()
            .exec
            .push_ok(script)
            .push_err(OxiError::forbidden("no exec"));

        let options = ExecOptions {
            stdin: true,
            ..ExecOptions::default()
        };
        let mut session =
            block_on(fake.exec("demo", "web", &argv(&["sh", "-c", "true"]), &options)).unwrap();
        assert!(session.resize.is_none());
        let out: Vec<u8> = block_on(session.stdout.take().unwrap().map(Result::unwrap).concat());
        assert_eq!(out, b"hello world");
        let err: Vec<u8> = block_on(session.stderr.take().unwrap().map(Result::unwrap).concat());
        assert_eq!(err, b"warn");
        block_on(send_all(
            session.stdin.as_mut().unwrap(),
            vec![b"ls\n".to_vec(), b"exit\n".to_vec()],
        ))
        .unwrap();
        assert_eq!(capture.stdin(), b"ls\nexit\n");
        assert_eq!(block_on(session.status).unwrap().code, Some(2));

        let denied = block_on(fake.exec("demo", "web", &argv(&["id"]), &options)).unwrap_err();
        assert_eq!(denied.kind(), ErrorKind::Forbidden);
        assert_eq!(
            fake.recorded_calls()[0],
            ExecCall::Exec {
                namespace: "demo".into(),
                pod: "web".into(),
                command: argv(&["sh", "-c", "true"]),
                options,
            }
        );
        assert_eq!(fake.recorded_calls().len(), 2);
    }

    #[test]
    fn attach_with_tty_captures_resizes_and_waits_for_finish() {
        let fake = FakeExecPort::new();
        let script = ExecScript::new().exit_when_told();
        let capture = script.capture();
        fake.script().attach.push_ok(script);
        let mut session =
            block_on(fake.attach("demo", "web", &ExecOptions::interactive())).unwrap();
        assert!(session.stderr.is_none(), "no stderr with a tty");
        block_on(send_all(
            session.resize.as_mut().unwrap(),
            vec![TerminalSize::new(80, 24), TerminalSize::new(120, 40)],
        ))
        .unwrap();
        assert_eq!(
            capture.resizes(),
            vec![TerminalSize::new(80, 24), TerminalSize::new(120, 40)]
        );
        let mut status = session.status;
        assert!((&mut status).now_or_never().is_none());
        assert!(capture.finish(Ok(ExitStatus::success())));
        assert!(block_on(status).unwrap().is_success());
        assert!(!capture.finish(Ok(ExitStatus::success())));
        assert!(matches!(fake.recorded_calls()[0], ExecCall::Attach { .. }));
        assert!(block_on(fake.attach("demo", "web", &ExecOptions::default())).is_err());
    }

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
