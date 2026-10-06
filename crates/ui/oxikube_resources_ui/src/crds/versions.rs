//! [`served_versions`]: the versions of one kind the cluster serves, for the table's switcher.

use oxikube_domain::kinds::{ResourceKind, Verb};

use super::info::version_order;

/// The listable versions of `of`'s kind (same API group and kind) among `discovered`, newest
/// first, `of` included. Discovery is the authority on what is served (a CRD may serve several
/// versions at once); a kind with one answer has nothing to switch.
pub fn served_versions(discovered: &[ResourceKind], of: &ResourceKind) -> Vec<ResourceKind> {
    let mut found: Vec<ResourceKind> = discovered
        .iter()
        .filter(|kind| {
            kind.gvk.group == of.gvk.group
                && kind.gvk.kind == of.gvk.kind
                && kind.supports(Verb::List)
        })
        .cloned()
        .collect();
    found.sort_by(|a, b| version_order(&a.gvk.version, &b.gvk.version));
    found.dedup_by(|a, b| a.gvk.version == b.gvk.version);
    found
}
