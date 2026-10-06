//! Exec, attach, node shells and debug containers over the websocket streaming protocol
//! (E04-S09).
//!
//! [`KubeExec`] is the adapter for one cluster. It implements
//! [`ExecStreamPort`] (raw streams) and [`oxikube_ports::ExecPort`] (terminal backends) on kube's
//! `Api<Pod>::exec` / `attach` and adds the two
//! helpers that get a user into a node or into a pod that has no shell:
//! [`KubeExec::node_shell`] and [`KubeExec::debug_container`]. There is no `kubectl` binary
//! involved anywhere. This crate stays free of terminal emulation: the session is raw byte
//! streams plus a resize sink; `ExecPort` hands them out as `TerminalBackend`s, which
//! `oxikube_terminal` drives.
//!
//! | Piece | Where |
//! |---|---|
//! | `ExecStreamPort` impl, opening the websocket | `mod` (this file) |
//! | `ExecPort` impl: descriptors to options, sessions to `TerminalBackend`s | `terminal` |
//! | `ExecOptions` to `AttachParams`, validation, pipe sizes | `params` |
//! | kube `AttachedProcess` to [`oxikube_ports::ExecSession`] | `session` |
//! | stdin as a `Sink`, output as chunk streams, resize as a `Sink` | `stdin`, `output`, `resize` |
//! | the status channel to [`oxikube_ports::ExitStatus`] | `status` |
//! | error mapping, sharpened by a pod read on the failure path | `error` |
//! | pod reads and writes behind a test seam, container readiness | `pods`, `wait` |
//! | privileged node pod: manifest, cleanup guard, leftover sweep | [`node_shell`] |
//! | ephemeral debug container | `debug` |
//!
//! # How a session behaves
//!
//! * **Streams.** `stdin`, `stdout`, `stderr` and `resize` are `Some` exactly when the
//!   [`oxikube_ports::ExecOptions`] asked for them (`resize` needs a TTY; the
//!   server rejects a TTY together with `stderr`, so that combination is refused here with a
//!   `Validation` error before any request). Output chunks are passed through as they arrive,
//!   one copy per chunk. Every pipe is bounded at 32 KiB: a consumer that stops reading stops
//!   the websocket task, and the server's flow control slows the container; a producer that
//!   writes faster than the container reads waits in `poll_ready`.
//! * **Ending.** `status` resolves when the command ends: [`oxikube_ports::ExitStatus`]
//!   with the exit code (`NonZeroExitCode` failures included, they are results, not errors). A
//!   connection that closes without a status is a result with no code; a websocket failure is a
//!   retryable `Network` error. Nothing reconnects: a closed interactive session is surfaced as
//!   closed, never silently re-established.
//! * **Ownership.** The `status` future owns the websocket task. Dropping the session, or just
//!   its `status`, aborts the task and closes the connection; the streams then end. Keep
//!   `status` for as long as the streams are in use.
//! * **Privacy.** The command, its arguments and everything on the streams stay out of logs and
//!   errors (they can carry secrets). A start is logged with the target only.
//!
//! # Policy
//!
//! Exec is privileged but is not a `MutationGuard` operation: callers gate it on the `exec`
//! capability and the session's read-only policy. Node shells and debug containers create
//! objects, so they are mutations: the callers that expose them run them through the guard
//! (the manifest and patch builders are public so the guard can show or dry-run what will be
//! sent), and the audit record carries the target and initiator, never the typed content.

mod debug;
mod error;
pub mod node_shell;
mod output;
mod params;
mod pods;
mod resize;
mod session;
mod status;
mod stdin;
mod terminal;
#[cfg(test)]
mod tests;
mod wait;

use std::sync::Arc;

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Pod;
use kube::{Api, Client};
use oxikube_domain::OxiResult;
use oxikube_ports::{ExecOptions, ExecSession, ExecStreamPort};

use crate::subresource::EphemeralContainerSpec;
use error::{Target, open_error, refine};
use params::{attach_params, check_command, check_target};
use pods::{KubePods, Pods};
use session::Parts;

pub use debug::DEFAULT_DEBUG_START_TIMEOUT;
pub use node_shell::{NodeShellConfig, NodeShellSession, node_shell_manifest};

/// Exec and attach on one connected cluster. Cheap to clone; clones share the client.
#[derive(Clone)]
pub struct KubeExec {
    client: Client,
    pods: Arc<dyn Pods>,
}

impl KubeExec {
    /// Executes through `client`.
    pub fn new(client: Client) -> Self {
        Self {
            pods: Arc::new(KubePods::new(client.clone())),
            client,
        }
    }

    /// Opens the websocket and wraps it. `command` is `None` for attach.
    async fn open(
        &self,
        namespace: &str,
        pod: &str,
        command: Option<&[String]>,
        options: &ExecOptions,
    ) -> OxiResult<ExecSession> {
        check_target(namespace, pod, options)?;
        if let Some(command) = command {
            check_command(command)?;
        }
        let params = attach_params(options);
        let api: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        let opened = match command {
            Some(command) => api.exec(pod, command.iter().cloned(), &params).await,
            None => api.attach(pod, &params).await,
        };
        let target = Target {
            namespace,
            pod,
            container: options.container.as_deref(),
            verb: if command.is_some() { "exec" } else { "attach" },
        };
        let process = match opened {
            Ok(process) => process,
            Err(err) => return Err(self.explain(&err, &target).await),
        };
        tracing::info!(
            namespace,
            pod,
            container = options.container.as_deref().unwrap_or(""),
            tty = options.tty,
            "{} session opened",
            target.verb
        );
        Ok(Parts::of(process).into_session())
    }

    /// The error for a failed open, sharpened with a pod read for the ambiguous statuses.
    async fn explain(&self, err: &kube::Error, target: &Target<'_>) -> oxikube_domain::OxiError {
        use oxikube_domain::ErrorKind::{NotFound, Validation};
        let first = open_error(err, target);
        if !matches!(first.kind(), NotFound | Validation) {
            return first;
        }
        match self.pods.shape(target.namespace, target.pod).await {
            Ok(shape) => refine(target, shape.as_ref()),
            // The lookup failing (RBAC on `get pods`, a dropped connection) leaves the
            // first answer standing.
            Err(_) => first,
        }
    }

    /// Opens a shell on `node` through a privileged pod pinned to it, and removes the pod when
    /// the shell ends, fails to open, or the session is dropped. See [`node_shell`].
    ///
    /// Creating the pod is a mutation (suggested guard tier: High).
    ///
    /// # Errors
    ///
    /// `Validation` for a bad node or configuration; the usual transport kinds, `Forbidden`
    /// when RBAC refuses the pod or the exec; `Conflict` when the pod cannot start (the image
    /// cannot be pulled, the node refuses it); `Timeout` when it does not start within
    /// [`NodeShellConfig::start_timeout`]. In every failure the pod is deleted before the
    /// error returns.
    pub async fn node_shell(
        &self,
        node: &str,
        config: &NodeShellConfig,
    ) -> OxiResult<NodeShellSession> {
        node_shell::open(self, self.pods.clone(), node, config).await
    }

    /// Deletes node-shell pods in `namespace` that an earlier run left behind (it crashed, or
    /// the connection dropped before the cleanup ran): those carrying Oxikube's node-shell
    /// label and older than `older_than`. Returns how many were deleted. See [`node_shell`].
    pub async fn sweep_node_shells(
        &self,
        namespace: &str,
        older_than: std::time::Duration,
    ) -> OxiResult<usize> {
        node_shell::sweep(self.pods.as_ref(), namespace, older_than).await
    }

    /// Adds the ephemeral container `spec` to `pod`, waits for it to run and attaches to it
    /// (`kubectl debug`). The container cannot be removed from the pod afterwards (a
    /// Kubernetes rule); it stays until the pod is replaced.
    ///
    /// Patching the pod is a mutation. `spec.stdin` and `spec.tty` decide the attached
    /// streams: set both for an interactive shell.
    ///
    /// # Errors
    ///
    /// `Validation` for a spec without a name or image, or one that changes the existing
    /// container of the same name (the same spec again just attaches to it);
    /// `NotFound`, `Forbidden` as for any pod patch; `Unsupported` when the cluster has no
    /// ephemeral containers; `Conflict` when the container cannot start; `Timeout` after
    /// `start_timeout` ([`DEFAULT_DEBUG_START_TIMEOUT`] is a good default).
    pub async fn debug_container(
        &self,
        namespace: &str,
        pod: &str,
        spec: &EphemeralContainerSpec,
        start_timeout: std::time::Duration,
    ) -> OxiResult<ExecSession> {
        debug::attach_debug(
            self,
            self.pods.as_ref(),
            namespace,
            pod,
            spec,
            start_timeout,
        )
        .await
    }
}

impl std::fmt::Debug for KubeExec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KubeExec").finish_non_exhaustive()
    }
}

#[async_trait]
impl ExecStreamPort for KubeExec {
    async fn exec_session(
        &self,
        namespace: &str,
        pod: &str,
        command: &[String],
        options: &ExecOptions,
    ) -> OxiResult<ExecSession> {
        self.open(namespace, pod, Some(command), options).await
    }

    async fn attach_session(
        &self,
        namespace: &str,
        pod: &str,
        options: &ExecOptions,
    ) -> OxiResult<ExecSession> {
        self.open(namespace, pod, None, options).await
    }
}
