//! Terminal sessions in containers, on nodes and in debug containers: [`ExecPort`] and the
//! [`TerminalBackend`] it hands out.
//!
//! The terminal must work the same whether bytes come from a local shell, a pod exec
//! session or a debug container. [`ExecPort`] opens the session and returns a
//! `Box<dyn TerminalBackend>`; the grid, the element and the tab only ever see the trait.
//!
//! | Piece | Where |
//! |---|---|
//! | [`TerminalBackend`], [`BackendEvent`], [`TerminalSize`], [`ExitStatus`] | `backend` |
//! | [`ExecTarget`], [`AttachTarget`], [`DebugContainerSpec`] | `target` |
//! | [`NodeShellSpec`] and the pod and command it renders to ([`node_shell_manifest`]) | `node_shell` |
//! | [`ExecStreamPort`]: the lower, stream-level exec ([`ExecSession`], [`ExecOptions`]) | `stream` |
//! | [`SessionBackend`]: an [`ExecSession`] as a [`TerminalBackend`] | `session_backend` |
//!
//! [`ExecStreamPort`] mirrors kube's `Api<Pod>::exec` / `attach` with `AttachParams`
//! (stdin writer, separate stdout/stderr readers, a resize sender, a status future). It
//! stays for callers that need raw streams (tar transfer, E20) and is what node shells and
//! debug containers are built on; [`ExecPort`] is the terminal-shaped view of the same
//! adapter (`oxikube_kube::remote::exec::KubeExec` implements both; `oxikube_terminal`
//! consumes the backends).

mod backend;
mod node_shell;
mod session_backend;
mod stream;
mod target;

use std::time::Duration;

use async_trait::async_trait;
use oxikube_domain::OxiResult;

pub use backend::{BackendEvent, ExitStatus, TerminalBackend, TerminalSize};
pub use node_shell::{
    CONTAINER_NAME as NODE_SHELL_CONTAINER, DEFAULT_NODE_SHELL_IMAGE, DEFAULT_NODE_SHELL_NAMESPACE,
    DEFAULT_NSENTER_ARGS, HEARTBEAT_ANNOTATION, NODE_ANNOTATION, NODE_SHELL_LABEL, NodeShellSpec,
    NodeShellToleration, node_shell_command, node_shell_manifest,
};
pub use session_backend::SessionBackend;
pub use stream::{ExecOptions, ExecSession, OutputStream, ResizeSink, StdinSink};
pub use target::{AttachTarget, DebugContainerSpec, ExecTarget};

/// Opens terminal sessions into a connected cluster: exec, attach, debug containers and
/// node shells. Each method returns a boxed [`TerminalBackend`] once the session is
/// established.
///
/// The byte streams may carry secrets; never log or persist them (non-negotiable 5).
///
/// # Effects
///
/// | Method | Effect |
/// |---|---|
/// | [`exec`](Self::exec) | none on the cluster (privileged: gate on the `exec` capability) |
/// | [`attach`](Self::attach) | none on the cluster (privileged: gate on the `exec` capability) |
/// | [`create_debug_container`](Self::create_debug_container) | **mutates**: patches the pod's `ephemeralcontainers` subresource; the container cannot be removed afterwards |
/// | [`node_shell`](Self::node_shell) | **mutates**: creates a privileged pod pinned to the node, deleted when the session ends |
/// | [`sweep_node_shells`](Self::sweep_node_shells) | **mutates**: deletes the node-shell pods whose owner is gone |
/// | [`release_node_shells`](Self::release_node_shells) | **mutates**: deletes the node-shell pods this port still has open |
///
/// `exec` and `attach` are not `MutationGuard` operations. `create_debug_container` and
/// `node_shell` are: `ExecService` (E09-S09, E09-S10) routes them through the guard
/// (confirmation tier, read-only mode, audit with target and initiator, never the typed
/// content). UI code never calls them directly.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// [`NotFound`](oxikube_domain::ErrorKind::NotFound) for an unknown pod, node or container,
/// [`Forbidden`](oxikube_domain::ErrorKind::Forbidden) when RBAC denies `pods/exec` (or the
/// pod create / patch),
/// [`Validation`](oxikube_domain::ErrorKind::Validation) for an empty command, a pod
/// reference without a namespace or a blank image,
/// [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) when the cluster refuses the stream
/// protocol or has no ephemeral containers,
/// [`Conflict`](oxikube_domain::ErrorKind::Conflict) when a helper container cannot start,
/// [`Network`](oxikube_domain::ErrorKind::Network) /
/// [`Timeout`](oxikube_domain::ErrorKind::Timeout) (retryable) for connection failures. A
/// command that runs and fails is an [`ExitStatus`] on the backend, not an error.
#[async_trait]
pub trait ExecPort: Send + Sync {
    /// Runs `target.command` (argv, no shell) in a container of `target.pod`.
    async fn exec(&self, target: &ExecTarget) -> OxiResult<Box<dyn TerminalBackend>>;

    /// Attaches to the main process of a container of `target.pod`.
    async fn attach(&self, target: &AttachTarget) -> OxiResult<Box<dyn TerminalBackend>>;

    /// Adds an ephemeral debug container to the pod, waits for it to run and attaches to it
    /// (`kubectl debug`). **Mutates the pod** (see the trait docs).
    async fn create_debug_container(
        &self,
        spec: &DebugContainerSpec,
    ) -> OxiResult<Box<dyn TerminalBackend>>;

    /// Opens a shell on a node through a privileged helper pod, which is deleted when the
    /// session ends, fails to open or the backend is dropped. **Creates a pod** (see the
    /// trait docs).
    async fn node_shell(&self, spec: &NodeShellSpec) -> OxiResult<Box<dyn TerminalBackend>>;

    /// Deletes the node-shell pods of `namespace` whose owner is gone: those with Oxikube's
    /// node-shell label that nobody has stamped alive (the pod's creation, or the
    /// [`HEARTBEAT_ANNOTATION`] its owner refreshes while the shell is open) for `older_than`.
    /// A shell that is open in another window, process or machine keeps stamping its pod, so it
    /// is spared however long it has been open. Returns the names of the pods that were deleted;
    /// one that fails to delete is skipped and logged. **Deletes pods** (see the trait docs).
    async fn sweep_node_shells(
        &self,
        namespace: &str,
        older_than: Duration,
    ) -> OxiResult<Vec<String>>;

    /// Deletes every node-shell pod this port opened whose shell is still open, and waits for the
    /// deletes (bounded). For the app's quit, which does not drop the terminals: without it the
    /// privileged pods would stay until their deadline. Returns how many it deleted; a delete
    /// that fails is logged, the sweep and the pod's deadline are the backstop. **Deletes pods**
    /// (see the trait docs).
    async fn release_node_shells(&self) -> OxiResult<usize>;
}

/// Stream-level exec and attach: the raw stdin/stdout/stderr/resize streams of a session.
///
/// Same privilege and error contract as [`ExecPort::exec`] / [`ExecPort::attach`], with
/// namespace and pod as strings. An adapter implements both ports; [`SessionBackend`] turns
/// an [`ExecSession`] into a [`TerminalBackend`].
///
/// # Effects
///
/// Privileged but not a `MutationGuard` operation: callers gate it on the session's exec
/// capability.
#[async_trait]
pub trait ExecStreamPort: Send + Sync {
    /// Runs `command` (argv, no shell) in a container of `pod`.
    async fn exec_session(
        &self,
        namespace: &str,
        pod: &str,
        command: &[String],
        options: &ExecOptions,
    ) -> OxiResult<ExecSession>;

    /// Attaches to the main process of a container of `pod`.
    async fn attach_session(
        &self,
        namespace: &str,
        pod: &str,
        options: &ExecOptions,
    ) -> OxiResult<ExecSession>;
}
