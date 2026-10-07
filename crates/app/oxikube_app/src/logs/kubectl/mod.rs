//! The kubectl fallback of the log viewer (E08-S08): the command line of `kubectl logs -f` for
//! what a view shows, and finding out whether kubectl is installed.
//!
//! Some users want kubectl exactly as they know it, or need a `kubectl logs` behaviour the viewer
//! does not cover. The viewer's "Tail in terminal" action runs it in a terminal tab. Plain Rust
//! (no gpui, no kube), so both halves are tested without a window or a cluster.
//!
//! | Piece | Where |
//! |---|---|
//! | the argv of `kubectl logs` for a pod or a selector, from the port's [`LogOptions`](oxikube_ports::LogOptions) | [`KubectlTail`], [`TailTarget`] (`argv`) |
//! | kubectl on this machine: the lookup, and the cached answer the UI reads | [`KubectlLookup`], [`PathLookup`], [`Kubectl`] (`locate`) |
//!
//! # No shell
//!
//! The command is an argv vector, never a string: the terminal backend runs the program with
//! these arguments directly, so a name with spaces, quotes or `$(...)` in it is one argument and
//! nothing is interpreted. Values are given as `--flag=value`, so a value that starts with `-`
//! cannot be taken for a flag, and a pod name that starts with `-` is refused.
//!
//! # No secrets
//!
//! The command line names the context, namespace, pod, container and selector, and nothing
//! secret. The cluster's credentials reach kubectl through the terminal's environment
//! (`KUBECONFIG`, a private merged file), never through the argv (non-negotiable 5).

mod argv;
mod locate;
#[cfg(test)]
mod tests;

pub use argv::{KubectlTail, TailTarget};
pub use locate::{Kubectl, KubectlLookup, PathLookup};
