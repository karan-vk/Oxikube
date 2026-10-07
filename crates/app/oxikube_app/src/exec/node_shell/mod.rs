//! Node shells (E09-S09): a shell on a node through a short-lived privileged pod.
//!
//! `node::Shell` is a *mutation* (it creates a pod that is root on the node), unlike the exec-class
//! `pod::Shell`, so it passes the whole [`MutationGuard`](crate::MutationGuard) pipeline: blocked
//! on a read-only cluster for every initiator, a confirmation that names the node and the image,
//! an audit record (`phase=create image=... namespace=...`). Two things happen after the guard
//! says yes:
//!
//! 1. **The guarded handler** ([`ExecService::authorize_node_shell`], called with the command's
//!    [`Mutation`](crate::Mutation)) renders the pod from the cluster's settings and runs it
//!    through the server as a *dry run*, so a missing permission, a pod security admission that
//!    refuses privileged pods or a quota fails here, with an actionable message and no terminal
//!    tab. It then leaves a one-shot permit for that node.
//! 2. **The terminal** ([`ExecService::open_node_shell`]) exchanges the permit for the session:
//!    the adapter creates the pod, waits for the shell container to run, execs `nsenter` into the
//!    node's namespaces and deletes the pod when the session ends, fails, is dropped or aborted.
//!    Without a permit nothing opens: the UI cannot reach the privileged port around the guard.
//!
//! When the terminal closes the service writes the second audit record (`phase=delete`) with the
//! same initiator, node, image and namespace, so both ends of the pod's life are on the trail.
//! Cleanup does not rely on the tab alone:
//!
//! * the adapter stamps the pod alive every minute while the shell is open, and deletes it when
//!   the shell ends, fails to open or is dropped;
//! * the app's quit does not drop the terminals, so [`ExecService::close_node_shells`] deletes
//!   the pods of the shells still open and writes their closing records and the audit backlog;
//! * leftovers of a run that died before cleaning up are swept the first time a cluster's node
//!   shell opens: pods with Oxikube's label that nothing has stamped for [`JANITOR_GRACE`], so
//!   another window's or user's live shell is never taken, however long it has been open. Each
//!   deletion is audited (`phase=sweep`);
//! * the pod's own `activeDeadlineSeconds` ends anything that remains.
//!
//! | Piece | Where |
//! |---|---|
//! | [`register_command`]: the `node::Shell` handler | `command` |
//! | [`ExecService::authorize_node_shell`], [`ExecService::open_node_shell`] | `service` |
//! | the one-shot permits | `permit` |
//! | the backend that audits the close | `backend` |
//! | the shells open now, ended on quit | `open_shells` |
//! | the leftover sweep and its audit | `sweep` |
//! | readable create and start failures | `failure` |

mod backend;
mod command;
mod failure;
mod open_shells;
mod permit;
mod service;
mod sweep;
#[cfg(test)]
mod tests;

pub use command::{NodeShellOpener, register_command};
pub(in crate::exec) use open_shells::OpenShells;
pub(in crate::exec) use permit::Permits;
pub use service::NodeShellPlan;
pub use sweep::JANITOR_GRACE;
