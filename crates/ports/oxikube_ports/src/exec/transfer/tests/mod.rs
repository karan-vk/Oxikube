//! Transfer tests over a scripted [`ExecStreamPort`]: no cluster, no runtime.

mod commands;
mod download;
mod upload;

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use futures::{FutureExt as _, Sink, StreamExt as _};
use oxikube_domain::{OxiError, OxiResult};
use parking_lot::Mutex;

use super::super::{ExecOptions, ExecSession, ExecStreamPort, ExitStatus, StdinSink};

/// Sets its flag when dropped: how a test sees that the session's `status` (which owns the
/// connection in the real adapter) was dropped before it resolved, i.e. the session was aborted.
struct AbortFlag {
    aborted: Arc<AtomicBool>,
    armed: bool,
}

impl Drop for AbortFlag {
    fn drop(&mut self) {
        if self.armed {
            self.aborted.store(true, Ordering::SeqCst);
        }
    }
}

/// One scripted session.
pub(super) struct Plan {
    pub(super) stdout: BoxStream<'static, OxiResult<Vec<u8>>>,
    pub(super) stderr: Vec<&'static str>,
    pub(super) status: Option<OxiResult<ExitStatus>>,
    pub(super) stdin: Option<StdinSink>,
}

impl Plan {
    pub(super) fn exits(code: i32) -> Self {
        Self {
            stdout: stream::empty().boxed(),
            stderr: Vec::new(),
            status: Some(Ok(ExitStatus::with_code(code))),
            stdin: None,
        }
    }

    pub(super) fn stdout_chunks(mut self, chunks: Vec<Vec<u8>>) -> Self {
        self.stdout = stream::iter(chunks.into_iter().map(Ok)).boxed();
        self
    }

    pub(super) fn stderr(mut self, line: &'static str) -> Self {
        self.stderr.push(line);
        self
    }

    /// The status never resolves (the program keeps running).
    pub(super) fn running(mut self) -> Self {
        self.status = None;
        self
    }
}

/// What the test sees of the session it scripted.
#[derive(Default)]
pub(super) struct Seen {
    pub(super) command: Mutex<Vec<String>>,
    pub(super) options: Mutex<Option<ExecOptions>>,
    pub(super) aborted: Arc<AtomicBool>,
}

impl Seen {
    pub(super) fn aborted(&self) -> bool {
        self.aborted.load(Ordering::SeqCst)
    }
}

/// An [`ExecStreamPort`] that opens one scripted session, or fails with `open_error`.
#[derive(Default)]
pub(super) struct Scripted {
    pub(super) plan: Mutex<Option<Plan>>,
    pub(super) open_error: Mutex<Option<OxiError>>,
    pub(super) seen: Arc<Seen>,
}

impl Scripted {
    pub(super) fn with(plan: Plan) -> Self {
        let port = Self::default();
        *port.plan.lock() = Some(plan);
        port
    }
}

#[async_trait]
impl ExecStreamPort for Scripted {
    async fn exec_session(
        &self,
        _namespace: &str,
        _pod: &str,
        command: &[String],
        options: &ExecOptions,
    ) -> OxiResult<ExecSession> {
        *self.seen.command.lock() = command.to_vec();
        *self.seen.options.lock() = Some(options.clone());
        if let Some(error) = self.open_error.lock().take() {
            return Err(error);
        }
        let plan = self.plan.lock().take().expect("one session scripted");
        let flag = AbortFlag {
            aborted: self.seen.aborted.clone(),
            armed: true,
        };
        let status = async move {
            // Moves the whole flag in, so dropping the future drops it.
            let mut flag = flag;
            let status = match plan.status {
                Some(status) => status,
                None => std::future::pending().await,
            };
            flag.armed = false;
            status
        }
        .boxed();
        let stderr = stream::iter(
            plan.stderr
                .into_iter()
                .map(|line| Ok(line.as_bytes().to_vec())),
        );
        Ok(ExecSession {
            stdin: plan.stdin,
            stdout: Some(plan.stdout),
            stderr: Some(stderr.boxed()),
            resize: None,
            status,
        })
    }

    async fn attach_session(&self, _: &str, _: &str, _: &ExecOptions) -> OxiResult<ExecSession> {
        Err(OxiError::unsupported("not scripted"))
    }
}

/// A stdin that keeps what it is sent and whether it was closed. It refuses writes past
/// `accept` bytes (a remote that stopped reading), and never gets ready when `blocked` (a remote
/// that is not reading yet).
#[derive(Clone)]
pub(super) struct Stdin {
    pub(super) bytes: Arc<Mutex<Vec<u8>>>,
    pub(super) closed: Arc<AtomicBool>,
    accept: usize,
    blocked: bool,
}

impl Stdin {
    pub(super) fn new() -> Self {
        Self {
            bytes: Arc::default(),
            closed: Arc::default(),
            accept: usize::MAX,
            blocked: false,
        }
    }

    pub(super) fn accepting(mut self, accept: usize) -> Self {
        self.accept = accept;
        self
    }

    pub(super) fn blocked(mut self) -> Self {
        self.blocked = true;
        self
    }

    pub(super) fn sink(&self) -> StdinSink {
        Box::pin(self.clone())
    }

    pub(super) fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

impl Sink<Vec<u8>> for Stdin {
    type Error = OxiError;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<OxiResult<()>> {
        if self.blocked {
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn start_send(self: Pin<&mut Self>, chunk: Vec<u8>) -> OxiResult<()> {
        let mut bytes = self.bytes.lock();
        if bytes.len() + chunk.len() > self.accept {
            return Err(OxiError::network(
                "the remote process is no longer reading stdin",
            ));
        }
        bytes.extend_from_slice(&chunk);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<OxiResult<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<OxiResult<()>> {
        self.closed.store(true, Ordering::SeqCst);
        Poll::Ready(Ok(()))
    }
}

impl Plan {
    /// Stdout stays open after its chunks (the program is still writing).
    pub(super) fn chain_pending(mut self) -> Self {
        self.stdout = self.stdout.chain(stream::pending()).boxed();
        self
    }
}
