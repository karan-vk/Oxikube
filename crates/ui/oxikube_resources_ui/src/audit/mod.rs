//! The audit log viewer (E19-S09): every guarded mutation attempt, newest first, in a virtualised
//! table with filters, a detail panel and an export.
//!
//! `audit::Open` (the palette's "Audit Log") opens [`AuditView`] as a tab of the active cluster's
//! tab (or the tab of the cluster the command names, with that cluster's filter applied);
//! `audit::Export` writes the filtered records to a JSONL or CSV file. Both read the local audit
//! log only and change no cluster, so neither has a guard tier and both run on a read-only
//! cluster.
//!
//! | Module | Holds |
//! |---|---|
//! | [`commands`] | [`AUDIT_COMMANDS`], [`AuditRequest`], [`AuditCommandSink`], [`register_commands`] |
//! | [`controller`] | [`AuditViews`], [`AuditHost`]: opens the viewer in a cluster tab and applies the requests |
//! | [`model`] | [`AuditModel`], [`AuditColumn`]: the loaded rows, the keyset cursor, the selection |
//! | [`view`] | [`AuditView`]: the workspace item |
//!
//! # Data, and what it costs
//!
//! The viewer never loads the log. It reads one keyset page ([`PAGE_SIZE`]) of what the filters
//! match, off the UI thread (`spawn_kube`), and the table asks for the next page when the visible
//! rows near the end of the loaded ones. Filters go into the store query, so its indices answer
//! them. New records arrive through [`AuditLog::appended`](oxikube_app::AuditLog::appended), not a
//! timer: the signal coalesces, the viewer reads the newest page once and prepends what is new
//! (when scrolled to the top) or shows an "N new" pill (when not). Cells are formatted for the
//! visible rows only.

pub mod commands;
pub mod controller;
pub mod model;
pub mod view;

#[cfg(test)]
mod tests;

pub use commands::{AUDIT_COMMANDS, AuditCommandSink, AuditRequest, register_commands};
pub use controller::{AuditHost, AuditViews, AuditViewsDeps};
pub use model::{AuditColumn, AuditModel, MAX_LOADED_RECORDS};
pub use oxikube_app::audit::query::PAGE_SIZE;
pub use view::{AuditView, AuditViewDeps, ITEM_KEY};
