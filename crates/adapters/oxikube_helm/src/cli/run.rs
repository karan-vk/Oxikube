//! Running `helm` as a child process: to completion ([`run`]) or line by line ([`stream`]).
//!
//! Every child is spawned with `kill_on_drop(true)`, so dropping the call's future (or the
//! output stream) kills `helm`; a deadline kills it too and fails with `Timeout`. Values go
//! to its stdin, never its command line. The environment is the app's own minus the `HELM_*`
//! variables that would silently redirect the command to another cluster, namespace or
//! identity, so only the flags this adapter passes decide where it goes.

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use futures::StreamExt as _;
use futures::stream::{self, BoxStream};
use oxikube_domain::helm::{HelmError, OutputLine};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::HelmOutput;
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWriteExt as _, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{Instant, timeout, timeout_at};

use super::errors::{CliFailure, Redactor, summary};

/// Environment variables Helm reads in place of flags; removed so the user's shell state can
/// never pick the cluster, namespace or credentials.
const SCRUBBED_ENV: &[&str] = &[
    "HELM_NAMESPACE",
    "HELM_KUBECONTEXT",
    "HELM_KUBETOKEN",
    "HELM_KUBEAPISERVER",
    "HELM_KUBEASUSER",
    "HELM_KUBEASGROUPS",
    "HELM_KUBECAFILE",
    "HELM_KUBEINSECURE_SKIP_TLS_VERIFY",
    "HELM_KUBETLS_SERVER_NAME",
    "HELM_DEBUG",
];

/// Stderr lines kept for the error summary of a streamed command.
const TAIL_LINES: usize = 20;

/// One `helm` command line and how to run it.
#[derive(Debug)]
pub(crate) struct Invocation {
    /// The arguments (flags first, then `--` and the positionals).
    pub(crate) args: Vec<String>,
    /// Written to stdin, then stdin is closed (the values YAML of `--values=-`).
    pub(crate) stdin: Option<String>,
    /// Kill the process after this long.
    pub(crate) timeout: Duration,
    /// Applied to every line read back.
    pub(crate) redactor: Redactor,
    /// The release the command is about, for error messages.
    pub(crate) release: Option<String>,
}

impl Invocation {
    /// `args` with no stdin, no release and only the shared redaction.
    pub(crate) fn new(args: Vec<String>, timeout: Duration) -> Self {
        Self {
            args,
            stdin: None,
            timeout,
            redactor: Redactor::none(),
            release: None,
        }
    }
}

pub(crate) fn command(binary: &Path, invocation: &Invocation) -> Command {
    let mut command = Command::new(binary);
    command
        .args(&invocation.args)
        .stdin(if invocation.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .kill_on_drop(true);
    for name in SCRUBBED_ENV {
        command.env_remove(name);
    }
    command
}

fn spawn(binary: &Path, invocation: &Invocation) -> Result<Child, CliFailure> {
    command(binary, invocation).spawn().map_err(|error| {
        CliFailure::Other(if error.kind() == io::ErrorKind::NotFound {
            HelmError::BinaryMissing.into()
        } else {
            OxiError::internal(format!("could not run helm: {error}"))
        })
    })
}

fn pipe_error(error: io::Error) -> CliFailure {
    CliFailure::Other(OxiError::internal(format!(
        "reading helm's output failed: {error}"
    )))
}

fn exit_failure(status: ExitStatus, stderr: &str, redactor: &Redactor) -> CliFailure {
    let mut text = summary(stderr, redactor);
    if text.is_empty() {
        text = format!("helm exited with {status}");
    }
    CliFailure::Exit {
        code: status.code(),
        summary: text,
    }
}

/// Runs `invocation` to completion and returns its stdout.
pub(crate) async fn run(binary: &Path, invocation: Invocation) -> Result<String, CliFailure> {
    let mut child = spawn(binary, &invocation)?;
    let pipe = child.stdin.take();
    let input = invocation.stdin.as_deref();
    let write = async move {
        if let (Some(mut pipe), Some(text)) = (pipe, input) {
            // A helm that exits before reading closes the pipe; its exit status says why.
            let _ = pipe.write_all(text.as_bytes()).await;
        }
    };
    let work = async {
        let ((), output) = tokio::join!(write, child.wait_with_output());
        output
    };
    // On timeout the future, and with it the child, is dropped: `kill_on_drop` kills it.
    let output = timeout(invocation.timeout, work)
        .await
        .map_err(|_| CliFailure::Timeout(invocation.timeout))?
        .map_err(pipe_error)?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(exit_failure(output.status, &stderr, &invocation.redactor))
}

/// The lines of a pipe, in order, without their line endings. Bytes that are not UTF-8 are
/// replaced rather than failing the stream.
fn lines<R>(reader: R, make: fn(String) -> OutputLine) -> BoxStream<'static, io::Result<OutputLine>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    stream::unfold(
        BufReader::new(reader).split(b'\n'),
        move |mut lines| async move {
            let line = lines.next_segment().await.transpose()?;
            let line = line.map(|bytes| {
                let text = String::from_utf8_lossy(&bytes);
                make(text.strip_suffix('\r').unwrap_or(&text).to_owned())
            });
            Some((line, lines))
        },
    )
    .boxed()
}

struct Running {
    child: Child,
    lines: BoxStream<'static, io::Result<OutputLine>>,
    stderr_tail: VecDeque<String>,
    deadline: Instant,
    invocation: Invocation,
}

enum State {
    Start(PathBuf, Invocation),
    Running(Box<Running>),
    Done,
}

/// Starts `invocation` on first poll and yields its stdout and stderr lines (redacted) as they
/// arrive, then ends, or yields one error when `helm` fails or times out. Dropping the stream
/// kills the process.
pub(crate) fn stream(binary: PathBuf, invocation: Invocation) -> HelmOutput {
    stream::unfold(State::Start(binary, invocation), |state| async move {
        let mut running = match state {
            State::Done => return None,
            State::Start(binary, invocation) => match start(&binary, invocation).await {
                Ok(running) => Box::new(running),
                Err((failure, release)) => {
                    return Some((Err(failure.into_error(release.as_deref())), State::Done));
                }
            },
            State::Running(running) => running,
        };
        match next(&mut running).await {
            Ok(Some(line)) => Some((Ok(line), State::Running(running))),
            Ok(None) => None,
            Err(failure) => {
                let error = failure.into_error(running.invocation.release.as_deref());
                Some((Err(error), State::Done))
            }
        }
    })
    .boxed()
}

async fn start(
    binary: &Path,
    mut invocation: Invocation,
) -> Result<Running, (CliFailure, Option<String>)> {
    let deadline = Instant::now() + invocation.timeout;
    let fail = |failure, invocation: &Invocation| (failure, invocation.release.clone());
    let mut child = spawn(binary, &invocation).map_err(|f| fail(f, &invocation))?;
    if let (Some(mut pipe), Some(text)) = (child.stdin.take(), invocation.stdin.take()) {
        // Helm reads `--values=-` before it does anything else, so writing first cannot stall.
        match timeout_at(deadline, pipe.write_all(text.as_bytes())).await {
            Err(_) => return Err(fail(CliFailure::Timeout(invocation.timeout), &invocation)),
            Ok(_ignored_epipe) => drop(pipe),
        }
    }
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let failure = CliFailure::Other(OxiError::internal("helm's output pipes are missing"));
        return Err(fail(failure, &invocation));
    };
    let lines = stream::select(
        lines(stdout, OutputLine::stdout),
        lines(stderr, OutputLine::stderr),
    );
    Ok(Running {
        child,
        lines: lines.boxed(),
        stderr_tail: VecDeque::new(),
        deadline,
        invocation,
    })
}

/// The next redacted line, `None` after a successful exit.
async fn next(running: &mut Running) -> Result<Option<OutputLine>, CliFailure> {
    let after = running.invocation.timeout;
    let timed_out = || CliFailure::Timeout(after);
    match timeout_at(running.deadline, running.lines.next()).await {
        Err(_) => Err(timed_out()),
        Ok(Some(Err(error))) => Err(pipe_error(error)),
        Ok(Some(Ok(mut line))) => {
            line.text = running.invocation.redactor.apply(&line.text);
            if line.kind == oxikube_domain::helm::OutputKind::Stderr {
                if running.stderr_tail.len() == TAIL_LINES {
                    running.stderr_tail.pop_front();
                }
                running.stderr_tail.push_back(line.text.clone());
            }
            Ok(Some(line))
        }
        Ok(None) => {
            let status = timeout_at(running.deadline, running.child.wait())
                .await
                .map_err(|_| timed_out())?
                .map_err(pipe_error)?;
            if status.success() {
                return Ok(None);
            }
            let stderr = running.stderr_tail.make_contiguous().join("\n");
            Err(exit_failure(status, &stderr, &running.invocation.redactor))
        }
    }
}

/// Maps a [`run`] result for a port method.
pub(crate) fn finish(
    result: Result<String, CliFailure>,
    release: Option<&str>,
) -> OxiResult<String> {
    result.map_err(|failure| failure.into_error(release))
}
