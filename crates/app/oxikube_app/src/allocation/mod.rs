//! Allocation views (E13-S11): how full a node or a namespace is, as numbers a panel can draw.
//!
//! A node's detail answers "how full is this node?" with, per resource, what the pods on it
//! **request**, what they are **limited** to and what they **use** right now, all against what
//! the node can allocate. A namespace's detail answers the same against its `ResourceQuota`s.
//! Both are an [`AllocationView`]: rows of [`AllocationRow`]s with the text a view prints already
//! made, so the panels draw and compute nothing, and a test can assert a number without a window.
//!
//! | Item | Holds |
//! |---|---|
//! | [`AllocationKind`] | what a row measures: CPU, memory, ephemeral storage, pods or a GPU resource, and how its amounts read |
//! | [`AllocationRow`], [`Usage`], [`UnsetCount`], [`BarModel`] | one resource: capacity, requests, limits, usage, the pods that set none, and the three-segment bar |
//! | [`AllocationView`] | the rows of a node ([`for_node`](AllocationView::for_node)) or a namespace ([`for_namespace`](AllocationView::for_namespace)) |
//! | [`UsageState`], [`MeasuredUsage`], [`node_usage`], [`namespace_usage`] | live usage from [`MetricsPort`](oxikube_ports::MetricsPort), or the reason there is none |
//!
//! # No second set of maths
//!
//! Requests, limits, allocatable, the unset-pod counts and the percents all come from the E13-S02
//! functions ([`NodeAllocation`](crate::NodeAllocation), [`NamespaceAllocation`](crate::NamespaceAllocation),
//! [`QuotaCaps`](oxikube_domain::utilisation::QuotaCaps), [`percent`](oxikube_domain::utilisation::percent));
//! this module only arranges them and writes them as text. The tests cross-check the numbers
//! against the S02 oracle fixture.
//!
//! # Absence is a state
//!
//! Requests and limits come from the pods' specs, so they are always there. Usage is separate:
//! with no metrics-server, RBAC denied or a failing call the usage segment of every row reads
//! `metrics unavailable: <reason>` ([`Usage::Unavailable`]), never a zero. A resource
//! metrics-server does not measure (ephemeral storage, GPUs) says so ([`Usage::NotReported`]).
//!
//! # Rules worth knowing
//!
//! * Limits above 100 % of a node's allocatable are normal (over-commitment), so a row only
//!   *notes* it ([`AllocationRow::limits_note`]); it is not an error. Limits above a namespace's
//!   `limits.*` quota are not normal (the quota was lowered, or the pods predate it): the note says
//!   so in other words.
//! * A namespace without quotas shows its totals and no bar (there is nothing to fill against);
//!   with quotas, each resource is measured against the tightest unscoped cap and scoped quotas
//!   are listed as left out (a sum over the whole namespace cannot be compared with them).
//! * Pods that hold no resources (`Succeeded`, `Failed`) are not counted, and pods not yet
//!   scheduled count towards their namespace but towards no node: the S02 rules.
//!
//! # Cost
//!
//! Building a view is a few dozen allocations over already-summed amounts. The sums themselves are
//! one pass over the pods of the subject (a node has about 110, a namespace up to a few
//! thousand: about 0.65 us a pod, see [`crate::metrics`]); callers run it when the pods change,
//! never per frame.

mod kind;
mod row;
mod usage;
mod view;

#[cfg(test)]
mod tests;

pub use kind::AllocationKind;
pub use row::{AllocationRow, BarBasis, BarModel, CapacityKind, UnsetCount, Usage};
pub use usage::{MeasuredUsage, UsageState, namespace_usage, node_usage};
pub use view::AllocationView;
