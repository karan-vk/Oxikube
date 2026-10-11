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

    /// The group without the kinds `claimed` (by group and kind) says another sidebar section
    /// lists already, such as the Gateway API under Network or VolumeSnapshots under Storage, so
    /// no kind is listed twice. `None` when every kind is claimed (the group is not shown).
    pub fn unclaimed(&self, claimed: impl Fn(&str, &str) -> bool) -> Option<CustomResourceGroup> {
        self.keep(|kind| !claimed(&self.group, &kind.kind))
    }

    /// The group as the sidebar's search shows it for `query` (trimmed; ASCII case is ignored):
    /// every kind when the group's name contains it, else the kinds whose kind or plural does.
    /// An empty query keeps the group whole. `None` when nothing matches.
    pub fn matching(&self, query: &str) -> Option<CustomResourceGroup> {
        let query = query.trim();
        if query.is_empty() || contains_ignore_case(&self.group, query) {
            return Some(self.clone());
        }
        self.keep(|kind| {
            contains_ignore_case(&kind.kind, query) || contains_ignore_case(&kind.plural, query)
        })
    }

    fn keep(&self, keep: impl Fn(&CustomKind) -> bool) -> Option<CustomResourceGroup> {
        let kinds: Vec<CustomKind> = self.kinds.iter().filter(|k| keep(k)).cloned().collect();
        (!kinds.is_empty()).then(|| CustomResourceGroup {
            group: self.group.clone(),
            kinds,
        })
    }
}

/// Whether `haystack` contains `needle`, ignoring ASCII case, without allocating (the sidebar
/// runs it over every custom kind on each keystroke of its search).
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    n.is_empty() || h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
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
