//! [`KubeStream`]: a kube `AttachedProcess` as a [`TerminalBackend`] (E09-S03).
//!
//! The websocket is opened by [`KubeExec`](super::KubeExec) (`Api<Pod>::exec` / `attach` with
//! `AttachParams` equivalent to `interactive_tty()` for a terminal) and turned into an
//! [`ExecSession`] by `session`. `KubeStream` drives that session through the port's
//! [`SessionBackend`] and remembers how it was opened, so the "Reconnect" action can open the
//! same target again ([`KubeStream::reconnect`]).
//!
//! | Backend call | Websocket |
//! |---|---|
//! | [`write`](TerminalBackend::write) | stdin channel (bounded pipe: waits while the container does not read) |
//! | [`resize`](TerminalBackend::resize) | resize channel, kube's `TerminalSize { width, height }` |
//! | [`output_stream`](TerminalBackend::output_stream) | stdout (with a TTY, the merged terminal output), then the status channel |
//! | [`kill`](TerminalBackend::kill), drop | ends the event stream, which closes the websocket (see Ownership) |
//!
//! # Ending
//!
//! The last event is [`BackendEvent::Exited`] when the server reported how the command ended
//! (`take_status`: `Success`, or `NonZeroExitCode` with its code). A connection that closes
//! without a status, or fails, is a drop rather than an exit: the stream ends with
//! [`BackendEvent::Error`] of kind `Network`, retryable. That is the cue for the terminal to
//! offer "Reconnect" (E09-S12). Nothing reconnects by itself: a new shell has none of the old
//! one's state, so the user decides.
//!
//! # Ownership
//!
//! Nothing here spawns: the only task is kube's websocket task, owned by the session's status
//! future. Dropping or killing the backend ends a consumer's event stream at its next poll and
//! drops that future, which aborts the task (`AttachedProcess`'s drop); a stream that was never
//! taken goes with the backend.

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    BackendEvent, ExecOptions, ExecSession, ExecStreamPort, SessionBackend, TerminalBackend,
    TerminalSize,
};

/// How a [`KubeStream`] was opened, to open it again.
#[derive(Clone)]
pub(super) struct Reopen {
    port: Arc<dyn ExecStreamPort>,
    namespace: String,
    pod: String,
    /// The argv for exec; `None` for attach.
    command: Option<Vec<String>>,
    options: ExecOptions,
}

impl Reopen {
    /// An exec of `command` in `namespace/pod`, through `port`.
    pub(super) fn exec(
        port: Arc<dyn ExecStreamPort>,
        namespace: &str,
        pod: &str,
        command: &[String],
        options: &ExecOptions,
    ) -> Self {
        Self {
            port,
            namespace: namespace.to_owned(),
            pod: pod.to_owned(),
            command: Some(command.to_vec()),
            options: options.clone(),
        }
    }

    /// An attach to `namespace/pod`, through `port`.
    pub(super) fn attach(
        port: Arc<dyn ExecStreamPort>,
        namespace: &str,
        pod: &str,
        options: &ExecOptions,
    ) -> Self {
        Self {
            port,
            namespace: namespace.to_owned(),
            pod: pod.to_owned(),
            command: None,
            options: options.clone(),
        }
    }

    async fn open(&self) -> OxiResult<ExecSession> {
        let Self {
            port,
            namespace,
            pod,
            command,
            options,
        } = self;
        match command {
            Some(command) => port.exec_session(namespace, pod, command, options).await,
            None => port.attach_session(namespace, pod, options).await,
        }
    }
}

/// A pod exec or attach session as a [`TerminalBackend`]: the backend
/// [`ExecPort`](oxikube_ports::ExecPort) on [`KubeExec`](super::KubeExec) hands out. See the
/// module docs for how each call maps onto the websocket and how a session ends.
pub struct KubeStream {
    session: SessionBackend,
    reopen: Option<Reopen>,
}

impl KubeStream {
    /// Drives `session`; `reopen` is how to open it again (`None` when that is not possible:
    /// a node shell's helper pod is gone with its session).
    pub(super) fn new(session: ExecSession, reopen: Option<Reopen>) -> Self {
        Self {
            session: SessionBackend::new(session),
            reopen,
        }
    }

    /// Whether [`reconnect`](Self::reconnect) can open this session's target again.
    pub fn can_reconnect(&self) -> bool {
        self.reopen.is_some()
    }

    /// Opens a new session to the same target (pod, container, command or attach, TTY), for
    /// the "Reconnect" action after a drop. This session is left as it is: drop it once the
    /// new one replaces it. The new process starts fresh; nothing of the old shell's state
    /// carries over.
    ///
    /// The caller re-checks policy first (`ExecService`): this only reopens.
    ///
    /// # Errors
    ///
    /// `Unsupported` for a node shell (open a new one instead, which creates a new helper
    /// pod); otherwise the errors of opening the session in the first place
    /// ([`ExecPort`](oxikube_ports::ExecPort)).
    pub async fn reconnect(&self) -> OxiResult<KubeStream> {
        let Some(reopen) = &self.reopen else {
            return Err(OxiError::unsupported(
                "this session cannot be reconnected; open a new one",
            ));
        };
        let session = reopen.open().await?;
        tracing::info!(
            namespace = %reopen.namespace,
            pod = %reopen.pod,
            container = reopen.options.container.as_deref().unwrap_or(""),
            "terminal session reconnected"
        );
        Ok(Self::new(session, Some(reopen.clone())))
    }
}

impl std::fmt::Debug for KubeStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the command: it can carry secrets.
        let mut debug = f.debug_struct("KubeStream");
        if let Some(reopen) = &self.reopen {
            debug
                .field("namespace", &reopen.namespace)
                .field("pod", &reopen.pod)
                .field("container", &reopen.options.container)
                .field("attach", &reopen.command.is_none());
        }
        debug.field("session", &self.session).finish()
    }
}

#[async_trait]
impl TerminalBackend for KubeStream {
    async fn write(&self, bytes: &[u8]) -> OxiResult<()> {
        self.session.write(bytes).await
    }

    async fn resize(&self, size: TerminalSize) -> OxiResult<()> {
        self.session.resize(size).await
    }

    fn output_stream(&self) -> BoxStream<'static, BackendEvent> {
        self.session.output_stream()
    }

    async fn kill(&self) -> OxiResult<()> {
        self.session.kill().await
    }
}
