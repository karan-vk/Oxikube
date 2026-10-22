//! Helm release resources (E16-S03): a release's rendered manifest mapped to `ResourceRef`s and
//! joined with the live objects in the [`ResourceStore`](crate::store::ResourceStore).
//!
//! ```text
//! HelmPort::manifest ──▶ parse_manifest (domain, spawn_blocking, cached per release+revision)
//!        ──▶ ResourcePlan::build (KindScopes from discovery: kind served? namespaced?)
//!        ──▶ ReleaseResourcesLease (ref-counted store subscriptions while the tab is open)
//!        ──▶ ReleaseResource { reference, state: Present{health} | Missing | Unknown }
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `service` | [`HelmService`]: manifest read and parsed once per `(release, revision, updated)`, the plan |
//! | `plan` | [`ResourcePlan`], [`PlannedResource`]: manifest objects as `ResourceRef`s, release-namespace defaulting, hooks and parse errors kept apart |
//! | `kinds` | [`KindScopes`]: namespaced or not, served version, from discovery (no hard-coded table) |
//! | `lease` | [`ReleaseResourcesLease`]: the store feeds the tab needs, released on drop |
//! | `join` | [`join`], [`ReleaseResource`], [`ResourceState`]: live object and health, or `Missing` / `Unknown` |
//!
//! # Semantics
//!
//! * A namespaced kind without `metadata.namespace` lives in the release namespace; a
//!   cluster-scoped kind has none. Whether a kind is namespaced comes from discovery.
//! * A manifest object the cluster lacks is [`ResourceState::Missing`] (flagged, never hidden),
//!   but only once the kind's feed has listed; before that it is `Unknown(NotLoaded)`. A kind
//!   discovery does not list (a CRD that is not installed) is `Unknown(KindNotServed)`.
//! * Health is `oxikube_domain::view::health_of` (pod phase, workload availability, ...); kinds
//!   without a rule are `Present { health: None }`. Nothing is derived twice.
//! * Hooks are not live resources: they are in [`ResourcePlan::hooks`]. Drift (manifest vs live
//!   spec) is out of scope.
//!
//! # Cost
//!
//! Parsing runs on a blocking thread and is cached; a lease holds one namespace-scoped feed per
//! kind and namespace (a feed another view already holds is shared, and the lookup reads the
//! cluster-wide feed first when one is open); [`join`] is one hash lookup per object. With the
//! lease dropped nothing runs.

mod join;
mod kinds;
mod lease;
mod plan;
mod service;

#[cfg(test)]
mod tests;

pub use join::{ReleaseResource, ResourceState, UnknownReason, join};
pub use kinds::{KindScopes, ServedKind};
pub use lease::ReleaseResourcesLease;
pub use plan::{PlannedResource, ResourcePlan};
pub use service::HelmService;
