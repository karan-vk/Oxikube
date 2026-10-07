//! Shells, attaches and commands in pod containers (E09-S08): [`ExecService`].
//!
//! `pod::Shell`, `pod::Attach` and `pod::Exec` open an interactive session in a container. The
//! UI wraps the [`TerminalBackend`](oxikube_ports::TerminalBackend) this service returns in a
//! terminal tab; node shells ([`node_shell`](ExecService::open_node_shell)) and debug containers
//! (E09-S10) reuse the service.
//!
//! | Piece | Where |
//! |---|---|
//! | [`ExecService`]: open a shell / attach / command, remember the last container per pod | `service` |
//! | [`PodContainers`], [`plan_container`], [`ContainerPlan`]: which containers can open and which one does | `containers` |
//! | the shell search: a quick `<shell> -c "exit 0"` per shell of the chain | `shell` |
//! | missing program vs failure, readable open errors, the no-shell advice | `failure` |
//! | the one-line notice at the top of the terminal | `notice` |
//! | node shells (E09-S09): the guarded `node::Shell` handler's dry run and permit, the terminal, the audit of its end, the leftover sweep | `node_shell` |
//!
//! # Policy
//!
//! These are *exec-class* commands ([`CommandMeta::exec`](oxikube_domain::command::CommandMeta)):
//! not mutations and never confirmed, but the [`MutationGuard`](crate::MutationGuard) blocks them
//! on a read-only cluster unless the cluster's `exec_in_read_only` setting allows it, and writes
//! one audit record per open (initiator, pod, container, program, never the content). Their MCP
//! tool stubs (`k8s.pod_shell`, `k8s.pod_attach`, `k8s.pod_exec`) are unsafe, interactive and
//! hidden from agents unless the user enables them. This service does none of that itself: it
//! only opens sessions, and is only reached through those commands.
//!
//! # The shell search
//!
//! `bash` is tried first, then `sh` (the `terminal.exec_shells` chain, like Lens). Each is
//! probed with a quick non-interactive exec instead of guessed from the image: exit 126/127 (or
//! `executable file not found`) means the container lacks it, and the next one is tried. When
//! none exists the error says so and points to a debug container (distroless images, Windows).

mod containers;
mod debug;
mod failure;
mod node_shell;
mod notice;
mod service;
mod shell;
#[cfg(test)]
mod tests;

pub use containers::{
    ContainerChoices, ContainerPlan, DEFAULT_CONTAINER_ANNOTATION, ExecContainer, PodContainers,
    container_to_open, plan_container,
};
pub use debug::{
    DEFAULT_DEBUG_START_TIMEOUT, DebugDefaults, DebugOpened, DebugPlan, DebugReport, DebugRequest,
    DebugRunner, check_name, plan_debug, split_command,
};
pub use node_shell::{JANITOR_GRACE, NodeShellOpener, NodeShellPlan, register_command};
pub use service::{ExecService, ShellOptions};
