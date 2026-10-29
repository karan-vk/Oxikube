//! [`FakeExecStreamPort`]: scripted exec and attach sessions at the stream level.
//!
//! Each scripted session comes with a capture handle ([`ExecCapture`]) the test keeps to inspect
//! what the app wrote (stdin, resizes), whether it closed stdin or dropped the session, and to
//! end it. The same fake drives the tar transfer helpers (`ExecTransferExt`, E20-S06): script a
//! download's archive as stdout ([`ExecScript::stdout_stream`] for a large one, produced only
//! as it is read), and read an upload's archive back from [`ExecCapture::stdin`].

use std::sync::Arc;

use async_trait::async_trait;
use futures::channel::oneshot;
use futures::stream::BoxStream;
use futures::{FutureExt, StreamExt, sink, stream};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ExecOptions, ExecSession, ExecStreamPort, ExitStatus, TerminalSize};
use parking_lot::Mutex;

use crate::script::{CallLog, Script};
use pipes::{CaptureStdin, Ended};

#[derive(Default)]
pub(super) struct ExecShared {
    pub(super) stdin: Vec<u8>,
    pub(super) stdin_len: u64,
    pub(super) discard_stdin: bool,
    pub(super) stdin_closed: bool,
    pub(super) resizes: Vec<TerminalSize>,
    pub(super) exit: Option<oneshot::Sender<OxiResult<ExitStatus>>>,
    pub(super) aborted: bool,
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
            .field("stdin_bytes", &shared.stdin_len)
            .field("stdin_closed", &shared.stdin_closed)
            .field("resizes", &shared.resizes)
            .field("aborted", &shared.aborted)
            .finish()
    }
}

impl ExecCapture {
    /// Every byte the app wrote to stdin, concatenated (empty with
    /// [`ExecScript::discard_stdin`]).
    pub fn stdin(&self) -> Vec<u8> {
        self.shared.lock().stdin.clone()
    }

    /// How many bytes the app wrote to stdin, kept or not.
    pub fn stdin_len(&self) -> u64 {
        self.shared.lock().stdin_len
    }

    /// Whether the app closed stdin (the remote program's end of input).
    pub fn stdin_closed(&self) -> bool {
        self.shared.lock().stdin_closed
    }

    /// Whether the app dropped the session's `status` before it resolved: on a real cluster that
    /// closes the connection and stops the remote process (a cancel).
    pub fn aborted(&self) -> bool {
        self.shared.lock().aborted
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

/// Output a session produces.
enum Output {
    Chunks(Vec<Vec<u8>>),
    Stream(BoxStream<'static, OxiResult<Vec<u8>>>),
}

impl Output {
    fn into_stream(self) -> BoxStream<'static, OxiResult<Vec<u8>>> {
        match self {
            Output::Chunks(chunks) => stream::iter(chunks.into_iter().map(Ok)).boxed(),
            Output::Stream(stream) => stream,
        }
    }
}

/// What one scripted exec or attach session does: the output it produces and how it
/// ends.
///
/// By default the session prints nothing and exits with code 0 as soon as the app awaits
/// `status`. Output is delivered immediately, chunk by chunk, in order.
pub struct ExecScript {
    stdout: Output,
    stderr: Vec<Vec<u8>>,
    status: Option<OxiResult<ExitStatus>>,
    shared: Arc<Mutex<ExecShared>>,
}

impl Default for ExecScript {
    fn default() -> Self {
        Self {
            stdout: Output::Chunks(Vec::new()),
            stderr: Vec::new(),
            status: Some(Ok(ExitStatus::success())),
            shared: Arc::default(),
        }
    }
}

impl std::fmt::Debug for ExecScript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stdout = match &self.stdout {
            Output::Chunks(chunks) => format!("{} chunks", chunks.len()),
            Output::Stream(_) => "a stream".to_owned(),
        };
        f.debug_struct("ExecScript")
            .field("stdout", &stdout)
            .field("stderr_chunks", &self.stderr.len())
            .finish_non_exhaustive()
    }
}

impl ExecScript {
    /// A session with no output that exits 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one stdout chunk. Replaces a [`stdout_stream`](Self::stdout_stream).
    #[must_use]
    pub fn stdout(mut self, chunk: impl Into<Vec<u8>>) -> Self {
        match &mut self.stdout {
            Output::Chunks(chunks) => chunks.push(chunk.into()),
            Output::Stream(_) => self.stdout = Output::Chunks(vec![chunk.into()]),
        }
        self
    }

    /// The session's stdout is `output`, read as the app polls it: a large download is produced
    /// lazily, so a test can count what was produced against what was consumed.
    #[must_use]
    pub fn stdout_stream(mut self, output: BoxStream<'static, OxiResult<Vec<u8>>>) -> Self {
        self.stdout = Output::Stream(output);
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

    /// Stdin is counted ([`ExecCapture::stdin_len`]) but not kept, for uploads of hundreds of
    /// megabytes.
    #[must_use]
    pub fn discard_stdin(self) -> Self {
        self.shared.lock().discard_stdin = true;
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
        let ended = Ended::new(shared.clone());
        let status = match status {
            Some(status) => futures::future::ready(status).boxed(),
            None => {
                let (tx, rx) = oneshot::channel();
                shared.lock().exit = Some(tx);
                rx.map(|r| r.unwrap_or_else(|_| Err(OxiError::network("exec session dropped"))))
                    .boxed()
            }
        };
        let status = async move {
            let mut ended = ended;
            let status = status.await;
            ended.resolve();
            status
        }
        .boxed();
        let resize = sink::unfold(shared.clone(), |shared, size: TerminalSize| async move {
            shared.lock().resizes.push(size);
            Ok::<_, OxiError>(shared)
        });
        ExecSession {
            stdin: options
                .stdin
                .then(|| Box::pin(CaptureStdin::new(shared)) as _),
            stdout: options.stdout.then(|| stdout.into_stream()),
            stderr: (options.stderr && !options.tty).then(|| Output::Chunks(stderr).into_stream()),
            resize: options.tty.then(|| Box::pin(resize) as _),
            status,
        }
    }
}

/// Queued sessions for each [`FakeExecStreamPort`] method.
#[derive(Debug, Default)]
pub struct ExecStreamScripts {
    /// `exec`.
    pub exec: Script<ExecScript>,
    /// `attach`.
    pub attach: Script<ExecScript>,
}

/// One call made on a [`FakeExecStreamPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecStreamCall {
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

/// Fake `ExecStreamPort`. Scripted only: each call takes the next [`ExecScript`]. The session
/// exposes stdin/stdout/stderr/resize exactly as the options ask (stderr is absent with a
/// TTY, as on a real cluster).
#[derive(Debug, Default)]
pub struct FakeExecStreamPort {
    script: ExecStreamScripts,
    calls: CallLog<ExecStreamCall>,
}

fake_plumbing!(FakeExecStreamPort, ExecStreamScripts, ExecStreamCall);

impl FakeExecStreamPort {
    /// A fake with nothing scripted.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ExecStreamPort for FakeExecStreamPort {
    async fn exec_session(
        &self,
        namespace: &str,
        pod: &str,
        command: &[String],
        options: &ExecOptions,
    ) -> OxiResult<ExecSession> {
        self.calls.record(ExecStreamCall::Exec {
            namespace: namespace.to_owned(),
            pod: pod.to_owned(),
            command: command.to_vec(),
            options: options.clone(),
        });
        let script = self
            .script
            .exec
            .next_or_unscripted("FakeExecStreamPort", "exec_session")?;
        Ok(script.into_session(options))
    }

    async fn attach_session(
        &self,
        namespace: &str,
        pod: &str,
        options: &ExecOptions,
    ) -> OxiResult<ExecSession> {
        self.calls.record(ExecStreamCall::Attach {
            namespace: namespace.to_owned(),
            pod: pod.to_owned(),
            options: options.clone(),
        });
        let script = self
            .script
            .attach
            .next_or_unscripted("FakeExecStreamPort", "attach_session")?;
        Ok(script.into_session(options))
    }
}

mod pipes;
#[cfg(test)]
mod tests;
