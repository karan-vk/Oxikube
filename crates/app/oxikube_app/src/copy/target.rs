//! What the target cluster must have for a copy: the kind, the apiVersion, the namespace.

use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::ResourceKind;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{DiscoveryPort, ResourceReader};

use super::refusal::Refusal;

/// The target's entry for `gvk`.
///
/// # Errors
///
/// A [`Refusal::VersionNotServed`] naming the versions the target does serve when it has the kind
/// in other versions only, a [`Refusal::KindNotServed`] when it has no such kind (a custom
/// resource whose definition the target lacks); discovery's own error otherwise.
pub(super) async fn served_kind(
    discovery: &dyn DiscoveryPort,
    gvk: &Gvk,
    context: &str,
) -> OxiResult<ResourceKind> {
    if let Some(kind) = discovery.resolve(gvk).await? {
        return Ok(kind);
    }
    let served: Vec<String> = discovery
        .discover()
        .await?
        .into_iter()
        .filter(|kind| kind.gvk.group == gvk.group && kind.gvk.kind == gvk.kind)
        .map(|kind| kind.gvk.api_version())
        .collect();
    let refusal = if served.is_empty() {
        Refusal::KindNotServed(format!(
            "{context} does not serve {} ({}): install its definition there first",
            gvk.kind,
            gvk.api_version()
        ))
    } else {
        Refusal::VersionNotServed(format!(
            "{context} does not serve {} as {}; it serves {}",
            gvk.kind,
            gvk.api_version(),
            served.join(", ")
        ))
    };
    Err(refusal.into_error())
}

/// Checks that the namespace of `target` exists on the target cluster. A cluster that will not
/// say (no right to read namespaces) passes: the write itself then fails if the namespace is
/// missing, with the server's message.
///
/// # Errors
///
/// A [`Refusal::NamespaceMissing`].
pub(super) async fn namespace_exists(
    reader: &dyn ResourceReader,
    target: &ResourceRef,
) -> OxiResult<()> {
    let Some(namespace) = target.namespace() else {
        return Ok(());
    };
    match reader.get_opt(&Gvk::namespace(), None, namespace).await {
        Ok(None) => Err(Refusal::NamespaceMissing(namespace.to_owned()).into_error()),
        Ok(Some(_)) => Ok(()),
        Err(error) if error.kind() == ErrorKind::Forbidden => Ok(()),
        Err(error) => Err(error),
    }
}

/// An error for a missing context.
pub(super) fn unavailable(what: &str) -> OxiError {
    OxiError::conflict(format!("{what} is not connected"))
}
