//! [`discover_custom_resources`]: the "Custom Resources" section's content.

use std::collections::BTreeMap;

use oxikube_domain::OxiResult;
use oxikube_domain::access::{AccessRequirement, is_builtin_api_group};
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::kinds::{ResourceKind, Verb};
use oxikube_ports::DiscoveryPort;

use crate::kinds::ServedKinds;
use crate::store::CountTarget;

/// One custom kind in a [`CustomResourceGroup`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CustomKind {
    /// The kind (`Application`).
    pub kind: String,
    /// The plural resource name (`applications`).
    pub plural: String,
    /// The preferred served version (`v1alpha1`): the one the sidebar opens and counts.
    pub version: String,
    /// Whether the objects live in a namespace (how the namespace selection applies to them).
    pub namespaced: bool,
}

impl CustomKind {
    /// The kind as a count target: what the `ResourceStore` reads when a feed for it is open.
    pub fn count_target(&self, group: &str) -> CountTarget {
        CountTarget::new(
            Gvk::new(group, self.version.as_str(), self.kind.as_str()),
            Scope::from_namespaced(self.namespaced),
        )
    }
}

/// The custom kinds of one API group (`argoproj.io`), sorted by kind. The sidebar's group row
/// shows how many kinds there are; counting the objects of each is the `ResourceStore`'s job and
/// only reads feeds that are open already.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomResourceGroup {
    /// The API group.
    pub group: String,
    /// The group's listable kinds.
    pub kinds: Vec<CustomKind>,
}

impl CustomResourceGroup {
    /// The group's kinds the user may list, `None` when there are none (the group is hidden).
    pub fn visible(
        &self,
        offers: impl Fn(&[AccessRequirement]) -> bool,
    ) -> Option<CustomResourceGroup> {
        let kinds: Vec<CustomKind> = self
            .kinds
            .iter()
            .filter(|kind| offers(&[AccessRequirement::list(&self.group, &kind.plural)]))
            .cloned()
            .collect();
        (!kinds.is_empty()).then(|| CustomResourceGroup {
            group: self.group.clone(),
            kinds,
        })
    }
}

/// Runs discovery and returns the kinds of non-built-in API groups, grouped by group (sorted),
/// preferred version only, listable kinds only.
///
/// # Errors
///
/// The discovery port's error.
pub async fn discover_custom_resources(
    discovery: &dyn DiscoveryPort,
) -> OxiResult<Vec<CustomResourceGroup>> {
    Ok(custom_resource_groups(&discovery.discover().await?))
}

/// What one discovery answer tells the sidebar (E12-S01): which kinds the cluster serves (the
/// registry's entries show only for those) and its custom resources.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SidebarDiscovery {
    /// Every kind the cluster serves, any version.
    pub served: ServedKinds,
    /// The custom resource kinds, grouped (see [`custom_resource_groups`]).
    pub custom: Vec<CustomResourceGroup>,
}

/// Runs discovery once for both halves of [`SidebarDiscovery`].
///
/// # Errors
///
/// The discovery port's error.
pub async fn discover_sidebar(discovery: &dyn DiscoveryPort) -> OxiResult<SidebarDiscovery> {
    let kinds = discovery.discover().await?;
    Ok(SidebarDiscovery {
        served: ServedKinds::from_kinds(&kinds),
        custom: custom_resource_groups(&kinds),
    })
}

/// The kinds of non-built-in API groups in `kinds`, grouped by group (sorted), preferred version
/// only, listable kinds only.
pub fn custom_resource_groups(kinds: &[ResourceKind]) -> Vec<CustomResourceGroup> {
    let mut groups: BTreeMap<String, Vec<CustomKind>> = BTreeMap::new();
    for kind in kinds {
        let group: &str = &kind.gvk.group;
        if kind.preferred && kind.supports(Verb::List) && !is_builtin_api_group(group) {
            groups
                .entry(group.to_owned())
                .or_default()
                .push(CustomKind {
                    kind: kind.gvk.kind.to_string(),
                    plural: kind.plural.clone(),
                    version: kind.gvk.version.to_string(),
                    namespaced: kind.namespaced,
                });
        }
    }
    groups
        .into_iter()
        .map(|(group, mut kinds)| {
            kinds.sort();
            kinds.dedup();
            CustomResourceGroup { group, kinds }
        })
        .collect()
}
