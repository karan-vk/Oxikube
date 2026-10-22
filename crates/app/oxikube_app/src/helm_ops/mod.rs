//! Rolling back and uninstalling a release (E16-S06): the guarded `helm::Rollback` and
//! `helm::Uninstall`.
//!
//! ```text
//! history row ─helm::Rollback──┐                      ┌─ read-only cluster: denied (audited)
//! header      ─helm::Uninstall─┴─> CommandBus ─> guard ┼─ confirm: simple (rollback, risk medium)
//! agent / palette: helm.rollback, helm.uninstall       │          type the name (uninstall, risk high)
//!                                                      └─> handler ─> HelmPort::{rollback, uninstall}
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `spec` | what the guard says about the two commands: confirmation text, audit target (`helm.sh/v3 Release ns/name`), audit detail, the name to type |
//! | `handler` | [`register_commands`]: both handlers (tool stubs `helm.rollback`, `helm.uninstall`) |
//! | `diff` | [`values_diff`]: the line diff of two revisions' values, for the rollback preview |
//!
//! # Guard
//!
//! Both are mutations: blocked on a read-only cluster for every initiator (the denial audited),
//! confirmed (rollback: one simple confirmation, risk medium; uninstall: the user types the
//! release name, risk high) and audited with the release, the revision and the flags, never
//! values or manifests. The UI and an agent run the same path; nothing reaches
//! [`HelmPort::rollback`](oxikube_ports::HelmPort::rollback) or
//! [`HelmPort::uninstall`](oxikube_ports::HelmPort::uninstall) except through these handlers.
//!
//! A dry-run dispatch asks the port for `helm ... --dry-run` (which Helm supports for both) and
//! changes nothing. The Releases view refreshes from the release-record watch after the run, not
//! by assuming it worked.

mod diff;
mod handler;
mod spec;

#[cfg(test)]
mod tests;

pub use diff::{DiffKind, DiffLine, values_diff};
pub use handler::register_commands;

pub(crate) use spec::{audit_detail, audit_target, expected_name, summary};
