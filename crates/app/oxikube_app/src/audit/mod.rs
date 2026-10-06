//! The audit trail of guarded mutations (E06-S02; hardened in E19).
//!
//! [`AuditLog`] is the one writer of [`AuditRecord`](oxikube_domain::audit::AuditRecord)s:
//! [`MutationGuard`](crate::guard::MutationGuard) hands it one record per mutation attempt
//! (allowed, denied, failed or cancelled) and it appends them through
//! [`StatePort::append_audit`](oxikube_ports::StatePort::append_audit), which the SQLite
//! adapter stores (ADR 0010).
//!
//! # Fail closed
//!
//! ADR 0012 makes the audit log mandatory, so a mutation must not run while the log
//! cannot be written:
//!
//! * a record that fails to append stays in an in-memory **backlog** (bounded by
//!   [`MAX_AUDIT_BACKLOG`]) and the mutation that produced it is reported as failed;
//! * before every mutation the guard calls [`AuditLog::ensure_writable`], which flushes
//!   the backlog first. While that fails the guard refuses the mutation without calling
//!   any port, so at most the one mutation whose own record failed ever runs unaudited
//!   on disk, and its record is written as soon as the store recovers.
//!
//! # Secrets
//!
//! Records carry no bodies by construction. The free-form `who` field is passed through
//! [`redact`](oxikube_domain::redact::redact) before it is stored (non-negotiable 5).

mod log;

#[cfg(test)]
mod tests;

pub use log::{AuditLog, MAX_AUDIT_BACKLOG};
