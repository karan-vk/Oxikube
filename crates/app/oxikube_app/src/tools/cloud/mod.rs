//! The cloud discovery tool for hosted agents (E18-S08): `cloud.discover_clusters`, over the
//! app's [`CloudDiscoveryService`].
//!
//! | Piece | Where |
//! |---|---|
//! | the read-only tool: cached clusters, refreshed only on request | [`DiscoverClustersTool`] |
//! | registering it | [`install`] |
//!
//! The tool is read-only (no [`risk`](oxikube_ports::ToolDef::risk), no `MutationGuard` tier).
//! By default it answers from the discovery cache and runs no CLI, so an agent cannot start a
//! storm of `aws`/`gcloud`/`az` processes; `refresh: true` runs one refresh of that provider
//! through the service, which applies the same cancellation and parallelism limits as the Cloud
//! section. A result holds names, locations, versions, status and whether the cluster is already
//! in the catalog; never an endpoint's credentials, exec arguments or tokens.
//!
//! The user-facing commands (`cloud::Refresh`, `cloud::Add`, `cloud::Remove`) are registered by
//! [`register_commands`](crate::cloud::register_commands); the bus gives each its own tool stub.
//! This tool is the agent's typed, read-only view of the same service.

mod discover;

use std::sync::Arc;

pub use discover::{DISCOVER_CLUSTERS, DiscoverClustersTool};

use super::{RegisterToolError, ToolRegistry};
use crate::cloud::CloudDiscoveryService;

/// Registers `cloud.discover_clusters` on `tools`, over `service`.
///
/// # Errors
///
/// [`RegisterToolError`] when the name is taken.
pub fn install(
    tools: &ToolRegistry,
    service: &CloudDiscoveryService,
) -> Result<(), RegisterToolError> {
    tools.register(Arc::new(DiscoverClustersTool::new(service.clone())))
}
