//! Downloads: the remote program's stdout as a [`ByteStream`], with stderr drained beside it.

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::future::BoxFuture;
use futures::{Future, Stream, StreamExt};
use oxikube_domain::{OxiError, OxiResult};

use super::super::{ExecOptions, ExecSession, ExecStreamPort, ExitStatus, OutputStream, StdinSink};
use super::failure::{Failure, Program, Tail};
use super::progress::ProgressMeter;
use super::target::{cancelled, cat_command, tar_create_command};
use super::{ByteStream, CancelledFuture, RemotePath, TransferOptions};

/// Opens `program` on `source` and returns its stdout as a stream.
pub(super) async fn open<P: ExecStreamPort + ?Sized>(
    port: &P,
    source: &RemotePath,
    program: Program,
    options: TransferOptions,
) -> OxiResult<ByteStream> {
    let command = match program {
        Program::Cat => cat_command(&source.path)?,
        Program::TarCreate | Program::TarExtract => tar_create_command(&source.path)?,
    };
    if options.cancel.is_cancelled() {
        return Err(cancelled());
    }
    let exec = ExecOptions {
        container: source.container.clone(),
        // Held open and never written: its end stops the remote tar (`STOP_ON_EOF`).
        stdin: program == Program::TarCreate,
        stdout: true,
        // Its own pipe, read beside stdout: a TTY would rewrite bytes (binary data corrupted)
        // and merge the two.
        stderr: true,
        tty: false,
    };
    let failure = Failure {
        program,
        target: source,
    };
    let session = port
        .exec_session(&source.namespace, &source.pod, &command, &exec)
        .await
        .map_err(|error| failure.open(error))?;
    Ok(DownloadStream::new(session, program, source.clone(), options).boxed())
}

/// The stdout of a running download as `Bytes`, one chunk per chunk the session reads (no copy:
/// each `Vec` becomes the `Bytes`), so memory stays at the session's bounded pipes plus the chunk
/// the consumer holds.
///
/// Each poll first drains whatever stderr has ready, so a chatty stderr can never fill its pipe
/// and stall stdout (the job a separate task does elsewhere, without needing a runtime here).
/// When stdout ends, the rest of stderr is read and the exit status awaited: exit 0 ends the
/// stream, anything else yields one typed error ([`Failure::exit`]) and ends it.
///
/// Cancelling the token, or dropping the stream, drops the session: the connection closes, and
/// the end of stdin stops the remote `tar` (the API server itself would leave it running; see
/// `STOP_ON_EOF`). After a cancel the stream yields one `Conflict`
/// error, so a consumer writing a file (`FsPort::write_stream`) abandons it instead of keeping a
/// truncated copy.
struct DownloadStream {
    /// Kept open, never written: dropping it is the remote end of input that stops the program.
    stdin: Option<StdinSink>,
    stdout: Option<OutputStream>,
    stderr: Option<OutputStream>,
    status: Option<BoxFuture<'static, OxiResult<ExitStatus>>>,
    tail: Tail,
    meter: ProgressMeter,
    cancel: CancelledFuture,
    program: Program,
    source: RemotePath,
    done: bool,
}

impl DownloadStream {
    fn new(
        session: ExecSession,
        program: Program,
        source: RemotePath,
        options: TransferOptions,
    ) -> Self {
        Self {
            stdin: session.stdin,
            stdout: session.stdout,
            stderr: session.stderr,
            status: Some(session.status),
            tail: Tail::default(),
            meter: ProgressMeter::new(options.progress, options.total_hint),
            cancel: options.cancel.cancelled(),
            program,
            source,
            done: false,
        }
    }

    /// Ends the stream: the session's parts are dropped, which closes the connection.
    fn end(&mut self, last: Option<OxiResult<Bytes>>) -> Poll<Option<OxiResult<Bytes>>> {
        self.done = true;
        self.stdin = None;
        self.stdout = None;
        self.stderr = None;
        self.status = None;
        self.meter.finish();
        Poll::Ready(last)
    }

    /// Reads every stderr chunk that is ready now.
    fn drain_stderr(&mut self, cx: &mut Context<'_>) {
        while let Some(stderr) = self.stderr.as_mut() {
            match stderr.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(chunk))) => self.tail.push(&chunk),
                // A broken stderr only loses the message; the status still tells the outcome.
                Poll::Ready(Some(Err(_)) | None) => self.stderr = None,
                Poll::Pending => break,
            }
        }
    }

    fn finish(&mut self, exit: OxiResult<ExitStatus>) -> Poll<Option<OxiResult<Bytes>>> {
        let failure = Failure {
            program: self.program,
            target: &self.source,
        };
        let outcome = exit.and_then(|status| failure.exit(&status, &self.tail));
        self.end(outcome.err().map(Err))
    }
}

impl Stream for DownloadStream {
    type Item = OxiResult<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(None);
        }
        if Pin::new(&mut this.cancel).poll(cx).is_ready() {
            return this.end(Some(Err(cancelled())));
        }
        this.drain_stderr(cx);
        if let Some(stdout) = this.stdout.as_mut() {
            match stdout.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    this.meter.advance(chunk.len());
                    return Poll::Ready(Some(Ok(Bytes::from(chunk))));
                }
                Poll::Ready(Some(Err(error))) => return this.end(Some(Err(error))),
                Poll::Ready(None) => this.stdout = None,
                Poll::Pending => return Poll::Pending,
            }
        }
        // Stdout ended: the program is done writing. Read the rest of stderr, then the status.
        if this.stderr.is_some() {
            this.drain_stderr(cx);
            if this.stderr.is_some() {
                return Poll::Pending;
            }
        }
        let Some(status) = this.status.as_mut() else {
            return this.end(Some(Err(OxiError::internal(
                "the download has no exit status",
            ))));
        };
        match status.as_mut().poll(cx) {
            Poll::Ready(exit) => this.finish(exit),
            Poll::Pending => Poll::Pending,
        }
    }
}
