//! The sessions of [`FakeTarExec`](super::FakeTarExec): one per transfer command.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::channel::{mpsc, oneshot};
use futures::{FutureExt as _, Sink, StreamExt as _, stream};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ExecOptions, ExecSession, ExitStatus};
use parking_lot::Mutex;

use super::State;
use super::archive::{self, UnpackError};
use super::fs::{Node, join, normalize};

/// The size of the stdout chunks, like the adapter's pipes.
const CHUNK: usize = 32 * 1024;

enum Command {
    TarCreate { parent: String, name: String },
    TarExtract { dir: String },
    Cat { file: String },
}

fn parse(command: &[String]) -> Option<Command> {
    let parts: Vec<&str> = command.iter().map(String::as_str).collect();
    match parts.as_slice() {
        ["sh", "-c", _, _, "tar", "-cf", "-", "-C", parent, name] => Some(Command::TarCreate {
            parent: (*parent).to_owned(),
            name: (*name).to_owned(),
        }),
        ["tar", "-xmf", "-", "-C", dir] => Some(Command::TarExtract {
            dir: (*dir).to_owned(),
        }),
        ["cat", file] => Some(Command::Cat {
            file: (*file).to_owned(),
        }),
        _ => None,
    }
}

/// How a command ended: what it printed on stderr and its exit code.
struct Outcome {
    stderr: String,
    code: i32,
}

impl Outcome {
    fn failed(stderr: String, code: i32) -> Self {
        Self { stderr, code }
    }
    fn missing(program: &str) -> Self {
        Self::failed(
            format!(
                "OCI runtime exec failed: exec: \"{program}\": executable file not found in $PATH"
            ),
            127,
        )
    }
}

fn is_denied(state: &State, path: &str) -> bool {
    let path = normalize(path);
    state
        .denied
        .iter()
        .any(|denied| path == *denied || path.starts_with(&format!("{denied}/")))
}

/// A finished session: `stdout` chunks, `stderr` text, the exit code.
fn finished(stdout: Vec<u8>, outcome: &Outcome, options: &ExecOptions) -> ExecSession {
    let chunks: Vec<OxiResult<Vec<u8>>> = stdout
        .chunks(CHUNK)
        .map(|chunk| Ok(chunk.to_vec()))
        .collect();
    let stderr = (!outcome.stderr.is_empty()).then(|| Ok(outcome.stderr.clone().into_bytes()));
    let status = if outcome.code == 0 {
        ExitStatus::success()
    } else {
        ExitStatus::with_code(outcome.code)
    };
    ExecSession {
        stdin: options.stdin.then(|| Box::pin(Discard) as _),
        stdout: options.stdout.then(|| stream::iter(chunks).boxed()),
        stderr: options.stderr.then(|| stream::iter(stderr).boxed()),
        resize: None,
        status: futures::future::ready(Ok(status)).boxed(),
    }
}

pub(super) fn open(
    state: &Arc<Mutex<State>>,
    command: &[String],
    options: &ExecOptions,
) -> OxiResult<ExecSession> {
    let Some(parsed) = parse(command) else {
        return Err(OxiError::validation(
            "FakeTarExec only runs the tar transfer commands (tar -cf, tar -xmf, cat)",
        ));
    };
    let guard = state.lock();
    match parsed {
        Command::TarCreate { parent, name } => {
            if guard.without_tar {
                return Ok(finished(Vec::new(), &Outcome::missing("tar"), options));
            }
            let root = join(&parent, &name);
            if is_denied(&guard, &root) {
                let said = format!("tar: {name}: Cannot open: Permission denied");
                return Ok(finished(Vec::new(), &Outcome::failed(said, 2), options));
            }
            if guard.fs.node(&root).is_none() {
                let said = format!("tar: {name}: Cannot stat: No such file or directory");
                return Ok(finished(Vec::new(), &Outcome::failed(said, 2), options));
            }
            let bytes = archive::build(&guard.fs, &parent, &name);
            Ok(finished(
                bytes,
                &Outcome {
                    stderr: String::new(),
                    code: 0,
                },
                options,
            ))
        }
        Command::Cat { file } => {
            if guard.without_cat {
                return Ok(finished(Vec::new(), &Outcome::missing("cat"), options));
            }
            if is_denied(&guard, &file) {
                let said = format!("cat: {file}: Permission denied");
                return Ok(finished(Vec::new(), &Outcome::failed(said, 1), options));
            }
            match guard.fs.node(&file) {
                Some(Node::File { data, .. }) => Ok(finished(
                    data.clone(),
                    &Outcome {
                        stderr: String::new(),
                        code: 0,
                    },
                    options,
                )),
                Some(Node::Dir { .. }) => {
                    let said = format!("cat: {file}: Is a directory");
                    Ok(finished(Vec::new(), &Outcome::failed(said, 1), options))
                }
                None => {
                    let said = format!("cat: {file}: No such file or directory");
                    Ok(finished(Vec::new(), &Outcome::failed(said, 1), options))
                }
            }
        }
        Command::TarExtract { dir } => {
            if guard.without_tar {
                return Ok(finished(Vec::new(), &Outcome::missing("tar"), options));
            }
            if !guard.fs.is_dir(&dir) {
                let said = format!("tar: {dir}: Cannot open: No such file or directory");
                return Ok(finished(Vec::new(), &Outcome::failed(said, 2), options));
            }
            drop(guard);
            Ok(extract_session(state.clone(), dir, options))
        }
    }
}

/// An upload: stdin is collected, and closing it unpacks the archive and settles the status.
fn extract_session(state: Arc<Mutex<State>>, dir: String, options: &ExecOptions) -> ExecSession {
    let (status_tx, status_rx) = oneshot::channel();
    let (stderr_tx, stderr_rx) = mpsc::unbounded();
    let sink = Collect {
        state,
        dir,
        buffer: Vec::new(),
        status: Some(status_tx),
        stderr: Some(stderr_tx),
    };
    ExecSession {
        stdin: options.stdin.then(|| Box::pin(sink) as _),
        stdout: None,
        stderr: options.stderr.then(|| stderr_rx.boxed()),
        resize: None,
        status: status_rx
            .map(|result| result.unwrap_or_else(|_| Err(OxiError::network("exec session dropped"))))
            .boxed(),
    }
}

/// Stdin that discards what it is sent (a download's held-open stdin).
struct Discard;

impl Sink<Vec<u8>> for Discard {
    type Error = OxiError;
    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<OxiResult<()>> {
        Poll::Ready(Ok(()))
    }
    fn start_send(self: Pin<&mut Self>, _: Vec<u8>) -> OxiResult<()> {
        Ok(())
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<OxiResult<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<OxiResult<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Stdin of an upload: collects the archive, unpacks it on close.
struct Collect {
    state: Arc<Mutex<State>>,
    dir: String,
    buffer: Vec<u8>,
    status: Option<oneshot::Sender<OxiResult<ExitStatus>>>,
    stderr: Option<mpsc::UnboundedSender<OxiResult<Vec<u8>>>>,
}

impl Collect {
    fn unpack(&self) -> Outcome {
        let mut state = self.state.lock();
        let read_only = state.read_only;
        let denied = state.denied.clone();
        let check = move |path: &str| -> Option<&'static str> {
            if read_only {
                return Some("Read-only file system");
            }
            let under = |denied: &String| path == denied || path.starts_with(&format!("{denied}/"));
            denied.iter().any(under).then_some("Permission denied")
        };
        match archive::unpack(&mut state.fs, &self.dir, &self.buffer, &check) {
            Ok(()) => Outcome {
                stderr: String::new(),
                code: 0,
            },
            Err(UnpackError::Corrupt) => Outcome::failed(
                "tar: This does not look like a tar archive\ntar: Exiting with failure status due to previous errors".into(),
                2,
            ),
            Err(UnpackError::UnsafeName(name)) => {
                Outcome::failed(format!("tar: {name}: Member name contains '..'"), 2)
            }
            Err(UnpackError::Refused(path, reason)) => {
                Outcome::failed(format!("tar: {path}: Cannot open: {reason}"), 2)
            }
        }
    }
}

impl Sink<Vec<u8>> for Collect {
    type Error = OxiError;
    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<OxiResult<()>> {
        Poll::Ready(Ok(()))
    }
    fn start_send(mut self: Pin<&mut Self>, bytes: Vec<u8>) -> OxiResult<()> {
        if self.status.is_none() {
            return Err(OxiError::network("stdin is closed"));
        }
        self.buffer.extend_from_slice(&bytes);
        Ok(())
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<OxiResult<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_close(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<OxiResult<()>> {
        if let Some(status) = self.status.take() {
            let outcome = self.unpack();
            if let Some(stderr) = self.stderr.take()
                && !outcome.stderr.is_empty()
            {
                let _ = stderr.unbounded_send(Ok(outcome.stderr.clone().into_bytes()));
            }
            let exit = if outcome.code == 0 {
                ExitStatus::success()
            } else {
                ExitStatus::with_code(outcome.code)
            };
            let _ = status.send(Ok(exit));
        }
        Poll::Ready(Ok(()))
    }
}
