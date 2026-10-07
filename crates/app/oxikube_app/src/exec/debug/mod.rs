//! Ephemeral debug containers (E09-S10): a tool-rich container added to a running pod (`kubectl
//! debug`), then a terminal attached to it once it runs.
//!
//! A distroless or minimal image has no shell, so `kubectl exec` fails exactly when debugging is
//! needed; an ephemeral container shares the pod's network and (with a target container) the
//! target's process namespace and brings its own tools.
//!
//! | File | Holds |
//! |---|---|
//! | `request` | [`DebugRequest`] (what the dialog or the `pod::Debug` command asks for), [`plan_debug`] / [`DebugPlan`] (defaults filled in, target and name checked against the pod), [`split_command`], [`check_name`] |
//! | `state` | `DebugState`: the last image used per cluster, and the sessions opened and not yet claimed by their terminal |
//! | `service` | `ExecService::debug_defaults`, `plan_debug` and `open_debug` |
//! | `runner` | [`DebugRunner`]: dispatches `pod::Debug` through the bus as the user and answers its confirmation |
//!
//! # Where the guard is
//!
//! Adding a container patches the pod, so it is a mutation: `pod::Debug` is a low-risk
//! [`MutationGuard`](crate::MutationGuard) command (blocked in read-only mode, a simple
//! confirmation that says the container cannot be removed, one audit record naming the image and
//! target, and a dry-run flag that stops after planning). [`ExecService::open_debug`] takes the
//! guard's [`Mutation`](crate::Mutation) as proof it is running inside that command, and checks
//! the read-only flag once more right before it patches, as the guard's writer does for a delete.
//!
//! # From the container to the terminal
//!
//! `open_debug` patches the pod, waits for the container to run (the adapter's wait, bounded by
//! the request's timeout, with the API server's own message when it refuses) and attaches. The
//! attached session is kept here under (pod, container) until the terminal tab that the
//! `pod::Debug` handler asks for claims it through [`ExecService::attach`]: the tab is a plain
//! pod-attach terminal, so *Reconnect* later attaches to the same container again instead of
//! adding another.
//!
//! # What cannot be undone
//!
//! Ephemeral containers cannot be removed or edited, and their names cannot be reused, until the
//! pod is deleted. Closing the tab ends the attach session only; when the shell exits, the
//! container's main process ends and it shows as Terminated in the pod.
//!
//! # Limits
//!
//! The `ExecPort` offers no server-side dry run of the `ephemeralcontainers` patch, so a dry-run
//! dispatch validates against the pod it reads (target, name, image) and stops there.

mod request;
mod runner;
mod service;
mod state;

pub use request::{
    DEFAULT_DEBUG_START_TIMEOUT, DebugPlan, DebugRequest, check_name, plan_debug, split_command,
};
pub use runner::{DebugReport, DebugRunner};
pub use service::{DebugDefaults, DebugOpened};
pub(super) use state::DebugState;
