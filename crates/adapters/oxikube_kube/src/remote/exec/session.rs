//! A kube `AttachedProcess` as an [`ExecSession`].

use futures::channel::mpsc;
use futures::future::BoxFuture;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;
use kube::api::{AttachedProcess, TerminalSize as KubeSize};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ExecSession, ExitStatus};
use tokio::io::{AsyncRead, AsyncWrite};

use super::output::chunks;
use super::resize::resize_sink;
use super::status::exit_status;
use super::stdin::StdinWriter;
use crate::auth::redacted_line;

/// The pieces of a running process, as plain I/O types, so everything between kube and the
/// port is testable without a websocket.
pub(super) struct Parts {
    pub(super) stdin: Option<Box<dyn AsyncWrite + Send + Unpin>>,
    pub(super) stdout: Option<Box<dyn AsyncRead + Send + Unpin>>,
    pub(super) stderr: Option<Box<dyn AsyncRead + Send + Unpin>>,
    pub(super) resize: Option<mpsc::Sender<KubeSize>>,
    /// The status the server sends when the command ends; `None` if the connection ended
    /// without one.
    pub(super) status: BoxFuture<'static, Option<Status>>,
    /// Waits for the websocket task to end and reports why it failed, if it did. Owns the
    /// process: dropping it aborts the task and closes the websocket.
    pub(super) finish: BoxFuture<'static, Result<(), String>>,
}

impl Parts {
    /// Takes every stream out of `process`.
    pub(super) fn of(mut process: AttachedProcess) -> Self {
        let stdin = process
            .stdin()
            .map(|w| Box::new(w) as Box<dyn AsyncWrite + Send + Unpin>);
        let stdout = process
            .stdout()
            .map(|r| Box::new(r) as Box<dyn AsyncRead + Send + Unpin>);
        let stderr = process
            .stderr()
            .map(|r| Box::new(r) as Box<dyn AsyncRead + Send + Unpin>);
        let resize = process.terminal_size();
        let status = process.take_status();
        Self {
            stdin,
            stdout,
            stderr,
            resize,
            status: match status {
                Some(status) => Box::pin(status),
                None => Box::pin(std::future::ready(None)),
            },
            finish: Box::pin(async move {
                process
                    .join()
                    .await
                    .map_err(|err| redacted_line(&err.to_string()))
            }),
        }
    }

    /// The session the port hands out.
    ///
    /// The `status` future owns the process, so dropping the session (or just its `status`)
    /// aborts the websocket task and closes the connection; the streams then end. Awaiting the
    /// future is not needed for the process to run.
    pub(super) fn into_session(self) -> ExecSession {
        let Self {
            stdin,
            stdout,
            stderr,
            resize,
            status,
            finish,
        } = self;
        ExecSession {
            stdin: stdin.map(|w| Box::pin(StdinWriter::new(w)) as _),
            stdout: stdout.map(chunks),
            stderr: stderr.map(chunks),
            resize: resize.map(resize_sink),
            status: Box::pin(async move {
                let status = status.await;
                let ended = finish.await;
                outcome(status, ended)
            }),
        }
    }
}

/// How the session ended: the server's status when it sent one; otherwise a closed
/// connection, which is a result and not an error unless the websocket task failed.
fn outcome(status: Option<Status>, ended: Result<(), String>) -> OxiResult<ExitStatus> {
    match (status, ended) {
        (Some(status), _) => Ok(exit_status(&status)),
        (None, Ok(())) => Ok(ExitStatus {
            message: Some("the connection closed without an exit status".into()),
            ..ExitStatus::default()
        }),
        (None, Err(reason)) => Err(OxiError::network(format!(
            "the connection to the container was lost: {reason}"
        ))
        .with_retryable(true)),
    }
}
