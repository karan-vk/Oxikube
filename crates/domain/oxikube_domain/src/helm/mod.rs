//! Helm vocabulary shared by the native release reader, the `helm` CLI adapter, the
//! `HelmService` and the Helm views (E16).
//!
//! Pure data: no I/O, no `helm` binary, no Kubernetes client. The port that moves these
//! types is `oxikube_ports::HelmPort`.
//!
//! | Module | Holds |
//! |---|---|
//! | [`catalog`] | [`ChartSummary`], [`ChartVersion`], [`group_hits`], [`compare_versions`]: search hits grouped per chart, newest version first |
//! | [`release`] | [`ReleaseRef`], [`ReleaseStatus`], [`ChartRef`], [`Release`], [`Revision`] |
//! | [`values`] | [`Values`]: a YAML/JSON values tree with `get` and Helm-style `merge` |
//! | [`repo`] | [`RepoEntry`], [`RepoAddRequest`], [`RepoPassword`], [`ChartHit`], [`SearchQuery`], [`ShowKind`] |
//! | [`request`] | [`InstallRequest`], [`UpgradeRequest`], [`RollbackRequest`], [`UninstallRequest`], [`OutputLine`] |
//! | [`capabilities`] | [`HelmCapabilities`], [`HelmVersion`]: what this session can do |
//! | [`gate`] | [`HelmGate`], [`HelmFeature`], [`GateReason`]: what the views, commands and tools may offer (E16-S11) |
//! | [`error`] | [`HelmError`] and its mapping onto [`OxiError`](crate::OxiError) |
//!
//! # Sensitivity
//!
//! Values and manifests can embed credentials. [`Values`] and the requests redact their
//! `Debug` output; callers show the data but never persist it (non-negotiable 5).

pub mod capabilities;
pub mod catalog;
pub mod error;
pub mod gate;
pub mod release;
pub mod repo;
pub mod request;
pub mod values;

pub use capabilities::{HelmCapabilities, HelmVersion};
pub use catalog::{ChartPart, ChartSummary, ChartVersion, compare_versions, group_hits};
pub use error::HelmError;
pub use gate::{FeatureUnavailable, GateReason, HelmFeature, HelmGate};
pub use release::{ChartRef, Release, ReleaseRef, ReleaseStatus, Revision};
pub use repo::{
    ChartHit, RepoAddRequest, RepoEntry, RepoPassword, SearchQuery, ShowKind, validate_repo,
};
pub use request::{
    InstallRequest, OutputKind, OutputLine, RollbackRequest, UninstallRequest, UpgradeRequest,
};
pub use values::Values;
