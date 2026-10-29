//! Uploads: a tar stream into `tar -xmf -` on the remote stdin, stderr drained beside it.

use futures::future::{self, Either, FusedFuture as _};
use futures::{FutureExt as _, SinkExt as _, StreamExt as _};
use oxikube_domain::{OxiError, OxiResult};

use super::super::{ExecOptions, ExecSession, ExecStreamPort, OutputStream, StdinSink};
use super::failure::{Failure, Program, Tail};
use super::progress::ProgressMeter;
use super::target::{cancelled, tar_extract_command};
use super::{ByteStream, RemotePath, TransferOptions};

/// Why feeding stdin stopped early.
enum FeedError {
    /// The local archive failed (a read error): the upload is abandoned.
    Source(OxiError),
    /// The remote stopped reading: its exit status says why (or that it was done).
    Sink,
}

/// Unpacks `archive` into the directory `dest` and returns the bytes sent.
pub(super) async fn upload<P: ExecStreamPort + ?Sized>(
    port: &P,
    dest: &RemotePath,
    archive: ByteStream,
    options: TransferOptions,
) -> OxiResult<u64> {
    let command = tar_extract_command(&dest.path)?;
    let cancel = options.cancel.clone();
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let work = run(port, dest, &command, archive, options).boxed();
    // A cancel drops `work`, and with it the session: the connection closes and the remote tar
    // stops (what it unpacked so far stays).
    match future::select(work, cancel.cancelled()).await {
        Either::Left((outcome, _)) => outcome,
        Either::Right(((), _)) => Err(cancelled()),
    }
}

async fn run<P: ExecStreamPort + ?Sized>(
    port: &P,
    dest: &RemotePath,
    command: &[String],
    archive: ByteStream,
    options: TransferOptions,
) -> OxiResult<u64> {
    let failure = Failure {
        program: Program::TarExtract,
        target: dest,
    };
    let exec = ExecOptions {
        container: dest.container.clone(),
        stdin: true,
        stdout: false,
        stderr: true,
        tty: false,
    };
    let ExecSession {
        stdin,
        stderr,
        status,
        ..
    } = port
        .exec_session(&dest.namespace, &dest.pod, command, &exec)
        .await
        .map_err(|error| failure.open(error))?;
    let stdin = stdin.ok_or_else(|| OxiError::internal("the upload session has no stdin"))?;
    let mut meter = ProgressMeter::new(options.progress, options.total_hint);
    let mut tail = Tail::default();
    {
        let drain = drain(stderr, &mut tail).fuse();
        let feed = feed(archive, stdin, &mut meter).fuse();
        futures::pin_mut!(drain, feed);
        let fed = loop {
            futures::select! {
                fed = feed => break fed,
                () = drain => {}
            }
        };
        match fed {
            // Dropping the session (on return) stops the remote tar.
            Err(FeedError::Source(error)) => return Err(error),
            // tar stops reading at the archive's end marker and may exit before the padding
            // after it was sent: a refused write is judged by the exit status below.
            Ok(()) | Err(FeedError::Sink) => {}
        }
        if !drain.is_terminated() {
            drain.await;
        }
    }
    let exit = status.await?;
    meter.finish();
    failure.exit(&exit, &tail)?;
    Ok(meter.bytes())
}

/// Sends every chunk of `archive` to `stdin`, then closes it (the remote's end of input).
async fn feed(
    mut archive: ByteStream,
    mut stdin: StdinSink,
    meter: &mut ProgressMeter,
) -> Result<(), FeedError> {
    while let Some(chunk) = archive.next().await {
        let chunk = chunk.map_err(FeedError::Source)?;
        let len = chunk.len();
        stdin
            .send(Vec::from(chunk))
            .await
            .map_err(|_| FeedError::Sink)?;
        meter.advance(len);
    }
    stdin.close().await.map_err(|_| FeedError::Sink)
}

/// Keeps the tail of `stderr` until it ends.
async fn drain(stderr: Option<OutputStream>, tail: &mut Tail) {
    let Some(mut stderr) = stderr else {
        return;
    };
    while let Some(Ok(chunk)) = stderr.next().await {
        tail.push(&chunk);
    }
}
