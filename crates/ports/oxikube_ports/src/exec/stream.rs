//! The stream-level exec session: kube's `AttachedProcess` as boxed `futures` sinks and
//! streams, so nothing above the adapter depends on kube or tokio.

use std::pin::Pin;

use futures::future::BoxFuture;
use futures::{Sink, Stream};
use oxikube_domain::{OxiError, OxiResult};

use super::{ExitStatus, TerminalSize};

/// Which streams to attach and whether to allocate a TTY. Mirrors kube
/// `AttachParams` (buffer sizes are adapter tuning and left out).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOptions {
    /// Target container; `None` means the pod's only (or default) container.
    pub container: Option<String>,
    /// Attach stdin.
    pub stdin: bool,
    /// Attach stdout.
    pub stdout: bool,
    /// Attach stderr. The server rejects `stderr` together with `tty`
    /// (a TTY merges both into stdout).
    pub stderr: bool,
    /// Allocate a TTY; required for [`ExecSession::resize`].
    pub tty: bool,
}

impl Default for ExecOptions {
    /// The server defaults: stdout and stderr, no stdin, no TTY.
    fn default() -> Self {
        Self {
            container: None,
            stdin: false,
            stdout: true,
            stderr: true,
            tty: false,
        }
    }
}

impl ExecOptions {
    /// An interactive terminal: stdin + stdout with a TTY, no separate stderr.
    pub fn interactive() -> Self {
        Self {
            container: None,
            stdin: true,
            stdout: true,
            stderr: false,
            tty: true,
        }
    }

    /// Sets the container.
    #[must_use]
    pub fn container(mut self, container: impl Into<String>) -> Self {
        self.container = Some(container.into());
        self
    }

    /// Whether the stream combination is one the server accepts: at least one
    /// stream, and not `stderr` with `tty`.
    pub fn is_valid(&self) -> bool {
        (self.stdin || self.stdout || self.stderr) && !(self.stderr && self.tty)
    }
}

/// Bytes written to the remote stdin.
pub type StdinSink = Pin<Box<dyn Sink<Vec<u8>, Error = OxiError> + Send>>;

/// Chunks read from the remote stdout or stderr. Ends when the stream closes.
pub type OutputStream = Pin<Box<dyn Stream<Item = OxiResult<Vec<u8>>> + Send>>;

/// Terminal resize requests (TTY sessions only).
pub type ResizeSink = Pin<Box<dyn Sink<TerminalSize, Error = OxiError> + Send>>;

/// A running exec or attach. Each field is `Some` exactly when the matching
/// [`ExecOptions`] flag asked for it.
///
/// Dropping the session closes the connection and stops the process's I/O.
pub struct ExecSession {
    /// Remote stdin; closing the sink sends EOF.
    pub stdin: Option<StdinSink>,
    /// Remote stdout (with a TTY, the merged terminal output).
    pub stdout: Option<OutputStream>,
    /// Remote stderr (never with a TTY).
    pub stderr: Option<OutputStream>,
    /// Resize channel (only with a TTY).
    pub resize: Option<ResizeSink>,
    /// Resolves when the process ends.
    pub status: BoxFuture<'static, OxiResult<ExitStatus>>,
}

impl std::fmt::Debug for ExecSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecSession")
            .field("stdin", &self.stdin.is_some())
            .field("stdout", &self.stdout.is_some())
            .field("stderr", &self.stderr.is_some())
            .field("resize", &self.resize.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use futures::{SinkExt, StreamExt};

    #[test]
    fn exec_options_defaults_and_validity() {
        let d = ExecOptions::default();
        assert!(d.stdout && d.stderr && !d.stdin && !d.tty);
        assert!(d.is_valid());

        let i = ExecOptions::interactive().container("app");
        assert_eq!(i.container.as_deref(), Some("app"));
        assert!(i.stdin && i.stdout && i.tty && !i.stderr);
        assert!(i.is_valid());

        let bad = ExecOptions {
            stderr: true,
            ..ExecOptions::interactive()
        };
        assert!(!bad.is_valid());
        let none = ExecOptions {
            container: None,
            stdin: false,
            stdout: false,
            stderr: false,
            tty: false,
        };
        assert!(!none.is_valid());
    }

    #[test]
    fn session_streams_are_usable() {
        let (stdin_tx, stdin_rx) = futures::channel::mpsc::unbounded::<Vec<u8>>();
        let (resize_tx, resize_rx) = futures::channel::mpsc::unbounded::<TerminalSize>();
        let mut session = ExecSession {
            stdin: Some(Box::pin(
                stdin_tx.sink_map_err(|e| OxiError::network(e.to_string())),
            )),
            stdout: Some(Box::pin(futures::stream::iter(vec![Ok(b"hi\n".to_vec())]))),
            stderr: None,
            resize: Some(Box::pin(
                resize_tx.sink_map_err(|e| OxiError::network(e.to_string())),
            )),
            status: Box::pin(async { Ok(ExitStatus::success()) }),
        };
        assert!(format!("{session:?}").contains("stderr: false"));
        block_on(async {
            let stdin = session.stdin.as_mut().expect("stdin");
            stdin.send(b"ls\n".to_vec()).await.expect("send");
            stdin.close().await.expect("close");
            let resize = session.resize.as_mut().expect("resize");
            resize
                .send(TerminalSize::new(120, 40))
                .await
                .expect("resize");
            resize.close().await.expect("close");
            let out: Vec<_> = session.stdout.take().expect("stdout").collect().await;
            assert_eq!(out.len(), 1);
            assert!(session.status.await.expect("status").is_success());
        });
        assert_eq!(
            block_on(stdin_rx.collect::<Vec<_>>()),
            vec![b"ls\n".to_vec()]
        );
        assert_eq!(
            block_on(resize_rx.collect::<Vec<_>>()),
            vec![TerminalSize::new(120, 40)]
        );
    }
}
