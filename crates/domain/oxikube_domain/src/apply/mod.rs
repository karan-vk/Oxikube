//! Getting manifests ready for the server (E20-S02): the order to apply them in and the fields
//! to strip before sending them.
//!
//! Pure functions over [`ManifestDoc`](crate::manifests::ManifestDoc) and JSON: no I/O, no
//! runtime. The app layer's apply service (E20-S03) consumes [`ApplyPlan`]; copy to another
//! cluster (E20-S08) consumes [`sanitize_for_apply`].
//!
//! | Module | Holds |
//! |---|---|
//! | `rank` | [`kind_rank`] and the one table of kind ranks |
//! | `plan` | [`ApplyPlan`], [`PlanItem`]: ordering, namespace defaulting, `depends_on` edges |
//! | `sanitize` | [`sanitize_for_apply`], [`SanitizeMode`], [`SanitizeReport`] |
//! | `strips` | the table of fields each kind loses in [`SanitizeMode::Copy`] |
//!
//! # Ordering is an approximation
//!
//! `kubectl apply -f` applies in file order and never reorders. The order here is *kubectl-style*:
//! the Helm `InstallOrder` / kustomize legacy ordering (namespaces and policies first, then
//! configuration, storage, definitions, RBAC, services, workloads, ingress, webhooks last), so a
//! buffer that would fail in file order usually applies. It is a heuristic, not kubectl's
//! behaviour; `depends_on` carries the hard edges.
//!
//! # Sensitivity
//!
//! A [`SanitizeReport`] holds paths only, never values: the object may be a Secret.

mod plan;
mod rank;
mod sanitize;
mod strips;

pub use plan::{ApplyPlan, PlanItem};
pub use rank::{UNKNOWN_KIND_RANK, kind_rank};
pub use sanitize::{SanitizeMode, SanitizeReport, sanitize_for_apply};

#[cfg(test)]
mod tests;
