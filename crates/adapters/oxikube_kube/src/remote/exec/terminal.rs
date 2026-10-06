//! [`ExecPort`] for [`KubeExec`]: descriptors to the stream-level calls, sessions to
//! [`KubeStream`]s.
//!
//! Everything here is mapping; the websocket, the node-shell pod and the debug patch are the
//! existing stream-level code. Each session is wrapped in a [`KubeStream`], which pulls output
//! through the session's bounded pipes (a stalled terminal stalls the container) and reads the
//! exit status when the output ends. Killing or dropping a node-shell backend drops the
//! session's status future, which deletes the helper pod.

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_ports::{
    AttachTarget, DebugContainerSpec, ExecOptions, ExecPort, ExecStreamPort, ExecTarget,
    NodeShellSpec, TerminalBackend,
};
use uuid::Uuid;

use super::kube_stream::{KubeStream, Reopen};
use super::{KubeExec, NodeShellConfig};
use crate::subresource::EphemeralContainerSpec;

/// The server rejects `stderr` with a TTY (a TTY merges both into stdout), so a TTY session
/// asks for stdout only.
fn options(container: Option<&str>, tty: bool, stdin: bool) -> ExecOptions {
    ExecOptions {
        container: container.map(str::to_owned),
        stdin,
        stdout: true,
        stderr: !tty,
        tty,
    }
}

/// The ephemeral container for a debug request: interactive (stdin and TTY), named
/// `debugger-<8 hex>` unless the caller chose a name.
pub(super) fn debug_spec(spec: &DebugContainerSpec) -> EphemeralContainerSpec {
    let name = spec.name.clone().unwrap_or_else(|| {
        let id = Uuid::new_v4().simple().to_string();
        format!("debugger-{}", &id[..8])
    });
    EphemeralContainerSpec {
        name,
        image: spec.image.clone(),
        command: spec.command.clone(),
        target_container: spec.target_container.clone(),
        stdin: true,
        tty: true,
    }
}

/// The node-shell configuration for a request: the spec's choices over the defaults.
pub(super) fn node_config(spec: &NodeShellSpec) -> NodeShellConfig {
    let defaults = NodeShellConfig::default();
    NodeShellConfig {
        namespace: spec.namespace.clone().unwrap_or(defaults.namespace),
        image: spec.image.clone().unwrap_or(defaults.image),
        image_pull_secret: spec.image_pull_secret.clone(),
        shell: spec.shell.clone(),
        start_timeout: spec.start_timeout,
        ..defaults
    }
}

impl KubeExec {
    fn port(&self) -> Arc<dyn ExecStreamPort> {
        Arc::new(self.clone())
    }

    /// Runs `target.command` (argv, no shell) in a container of `target.pod`, as the concrete
    /// [`KubeStream`] ([`ExecPort::exec`] boxes the same thing). With `target.tty` and
    /// `target.stdin` this is kube's `AttachParams::interactive_tty()`.
    ///
    /// # Errors
    ///
    /// As [`ExecPort::exec`].
    pub async fn exec_stream(&self, target: &ExecTarget) -> OxiResult<KubeStream> {
        let (namespace, pod) = target.namespaced_pod()?;
        let options = options(target.container.as_deref(), target.tty, target.stdin);
        let session = self
            .exec_session(namespace, pod, &target.command, &options)
            .await?;
        let reopen = Reopen::exec(self.port(), namespace, pod, &target.command, &options);
        Ok(KubeStream::new(session, Some(reopen)))
    }

    /// Attaches to the main process of a container of `target.pod`, as the concrete
    /// [`KubeStream`] ([`ExecPort::attach`] boxes the same thing).
    ///
    /// # Errors
    ///
    /// As [`ExecPort::attach`].
    pub async fn attach_stream(&self, target: &AttachTarget) -> OxiResult<KubeStream> {
        let (namespace, pod) = target.namespaced_pod()?;
        let options = options(target.container.as_deref(), target.tty, target.stdin);
        let session = self.attach_session(namespace, pod, &options).await?;
        let reopen = Reopen::attach(self.port(), namespace, pod, &options);
        Ok(KubeStream::new(session, Some(reopen)))
    }
}

#[async_trait]
impl ExecPort for KubeExec {
    async fn exec(&self, target: &ExecTarget) -> OxiResult<Box<dyn TerminalBackend>> {
        Ok(Box::new(self.exec_stream(target).await?))
    }

    async fn attach(&self, target: &AttachTarget) -> OxiResult<Box<dyn TerminalBackend>> {
        Ok(Box::new(self.attach_stream(target).await?))
    }

    async fn create_debug_container(
        &self,
        spec: &DebugContainerSpec,
    ) -> OxiResult<Box<dyn TerminalBackend>> {
        let (namespace, pod) = spec.namespaced_pod()?;
        let container = debug_spec(spec);
        let session = self
            .debug_container(namespace, pod, &container, spec.start_timeout)
            .await?;
        // Reconnecting attaches to the same ephemeral container again (it stays in the pod).
        let attach = options(Some(&container.name), container.tty, container.stdin);
        let reopen = Reopen::attach(self.port(), namespace, pod, &attach);
        Ok(Box::new(KubeStream::new(session, Some(reopen))))
    }

    async fn node_shell(&self, spec: &NodeShellSpec) -> OxiResult<Box<dyn TerminalBackend>> {
        let shell = KubeExec::node_shell(self, &spec.node, &node_config(spec)).await?;
        // The helper pod goes with the session: a reconnect would have nothing to reach.
        Ok(Box::new(KubeStream::new(shell.session, None)))
    }
}
