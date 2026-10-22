//! Helm vocabulary shared by the native release reader, the `helm` CLI adapter, the
//! `HelmService` and the Helm views (E16).
//!
//! Pure data: no I/O, no `helm` binary, no Kubernetes client. The port that moves these
//! types is `oxikube_ports::HelmPort`.
//!
//! | Module | Holds |
//! |---|---|
//! | [`catalog`] | [`ChartSummary`], [`ChartVersion`], [`group_hits`], [`compare_versions`]: search hits grouped per chart, newest version first |
//! | [`manifest`] | [`parse_manifest`]: a rendered manifest split into [`ManifestObject`]s, hooks apart, per-document errors (E16-S03) |
//! | [`mask`] | [`mask_values`], [`mask_notes`], [`is_sensitive_key`], [`MASK`]: credential-looking values hidden until revealed (E16-S05) |
//! | [`rows`] | [`ReleaseColumn`], [`sort_releases`]: the Releases table's columns and order (E16-S05) |
//! | [`release`] | [`ReleaseRef`], [`ReleaseStatus`], [`ChartRef`], [`Release`], [`Revision`] |
//! | [`values`] | [`Values`]: a YAML/JSON values tree with `get` and Helm-style `merge` |
//! | [`repo`] | [`RepoEntry`], [`RepoAddRequest`], [`RepoPassword`], [`ChartHit`], [`SearchQuery`], [`ShowKind`] |
//! | [`request`] | [`InstallRequest`], [`UpgradeRequest`], [`RollbackRequest`], [`UninstallRequest`], [`OutputLine`] |
//! | [`install`] | [`validate_release_name`], [`validate_namespace_name`], [`ValuesYaml`] (the values text of `helm::Install`, redacted `Debug`), [`ManifestSummary`] (objects per kind of a rendered manifest) |
//! | [`upgrade`] | [`ResourceChanges`], [`ObjectKey`] (what an upgrade adds, removes and changes, by kind and name), [`mask_values_for_diff`] (secret-looking values fingerprinted for a values diff; E16-S10) |
//! | [`values_docs`] | [`ValuesDocs`], [`KeyDoc`]: the values' documentation from `values.yaml` comments (helm-docs `# --`) and `values.schema.json`, for the values editor |
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
pub mod install;
pub mod manifest;
pub mod mask;
pub mod release;
pub mod repo;
pub mod request;
pub mod rows;
pub mod upgrade;
pub mod values;
pub mod values_docs;

pub use capabilities::{HelmCapabilities, HelmVersion};
pub use catalog::{ChartPart, ChartSummary, ChartVersion, compare_versions, group_hits};
pub use error::HelmError;
pub use gate::{FeatureUnavailable, GateReason, HelmFeature, HelmGate};
pub use install::{
    MAX_NAMESPACE_NAME, MAX_RELEASE_NAME, ManifestSummary, ValuesYaml, validate_namespace_name,
    validate_release_name,
};
pub use manifest::{ManifestError, ManifestObject, ParsedManifest, parse_manifest};
pub use mask::{MASK, is_sensitive_key, mask_notes, mask_values};
pub use release::{ChartRef, Release, ReleaseRef, ReleaseStatus, Revision};
pub use repo::{
    ChartHit, RepoAddRequest, RepoEntry, RepoPassword, SearchQuery, ShowKind, validate_repo,
};
pub use request::{
    InstallRequest, OutputKind, OutputLine, RollbackRequest, UninstallRequest, UpgradeRequest,
};
pub use rows::{ReleaseColumn, sort_releases};
pub use upgrade::{ObjectKey, ResourceChanges, mask_values_for_diff};
pub use values::Values;
pub use values_docs::{KeyDoc, ValuesDocs};
