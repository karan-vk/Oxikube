//! Looking a kind up through discovery, for both backends.

use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::ResourceKind;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::DiscoveryPort;

/// The kind `gvk` names, with its plural (which a `ResourceRef` does not carry). `Unsupported`
/// when the cluster does not serve it.
pub(crate) async fn resolve_kind(
    discovery: &dyn DiscoveryPort,
    gvk: &Gvk,
) -> OxiResult<ResourceKind> {
    discovery
        .resolve(gvk)
        .await?
        .ok_or_else(|| OxiError::unsupported(format!("the cluster does not serve {gvk}")))
}
