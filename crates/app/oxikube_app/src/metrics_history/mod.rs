//! History graphs for the Metrics tab of a detail panel (E13-S08): which charts each kind has,
//! how a range is fetched, and what the view gets back.
//!
//! | File | Holds |
//! |---|---|
//! | `subject` | [`SubjectKind`] (the six kinds with graphs) and [`HistorySubject`], one object with the names its queries filter on |
//! | `charts` | [`charts_for`]: the charts of a subject as data ([`ChartDef`]: measures and reference lines), one table for all kinds |
//! | `data` | [`ChartData`], [`SeriesData`], [`ReferenceLine`]: a fetched chart in base units, plus the helpers that build them from a `PromqlSeries` result |
//! | `service` | [`MetricsHistoryService`]: availability, the saved range, and [`fetch`](MetricsHistoryService::fetch), which builds each query with the stack's catalogue and runs them at once |
//!
//! # Rules
//!
//! * No polling here: the view asks when its tab is shown and on its own timer, and cancels by
//!   dropping the task. Nothing is queried for a panel nobody looks at.
//! * Absence is visible, never a zero: a cluster without an endpoint is
//!   [`HistoryAvailability::Unavailable`] with the reason; a failed query is a [`HistoryError`]
//!   whose text reads `metrics unavailable: <reason>`; an endpoint that has no series for the
//!   object is an empty [`ChartData`], which the view names as "no data for this object".
//! * Per-container series come from the catalogue's container scope (one series per
//!   container). A workload draws its first [`MAX_WORKLOAD_PODS`] pods, one line each.
//! * Requests, limits and capacity are the last reading of the reference queries. Values that
//!   are the same in every series (a workload's pods) become one line; different ones are named
//!   after their container or pod. A reference that fails to load leaves its line out, not the
//!   chart.
//! * Values stay in base units; the view formats them with `oxikube_ui::chart::ValueFormat`.

mod charts;
mod data;
mod service;
mod subject;
#[cfg(test)]
mod tests;

pub use charts::{ChartDef, Measure, Reference, charts_for};
pub use data::{ChartData, HistoryError, ReferenceKind, ReferenceLine, Sample, SeriesData};
pub use service::{HistoryAvailability, HistorySource, MetricsHistoryService};
pub use subject::{HistorySubject, MAX_WORKLOAD_PODS, SubjectKind};
