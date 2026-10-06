//! [`discover_custom_resources`]: the "Custom Resources" section's content.

use std::collections::BTreeMap;

use oxikube_domain::OxiResult;
use oxikube_domain::access::{AccessRequirement, is_builtin_api_group};
use oxikube_domain::kinds::Verb;
use oxikube_ports::DiscoveryPort;

/// One custom kind in a [`CustomResourceGroup`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CustomKind {
    /// The kind (`Application`).
    pub kind: String,
    /// The plural resource name (`applications`).
    pub plural: String,
}

/// The custom kinds of one API group (`argoproj.io`), sorted by kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomResourceGroup {
    /// The API group.
    pub group: String,
    /// The group's listable kinds.
    pub kinds: Vec<CustomKind>,
}

impl CustomResourceGroup {
    /// The access a kind of this group needs to be shown.
    pub fn requirement(&self, kind: &CustomKind) -> AccessRequirement {
        AccessRequirement::list(self.group.clone(), kind.plural.clone())
    }

    /// The group's kinds the user may list, `None` when there are none (the group is hidden).
    pub fn visible(
        &self,
        offers: impl Fn(&[AccessRequirement]) -> bool,
    ) -> Option<CustomResourceGroup> {
        let kinds: Vec<CustomKind> = self
            .kinds
            .iter()
            .filter(|kind| offers(&[self.requirement(kind)]))
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
    let mut groups: BTreeMap<String, Vec<CustomKind>> = BTreeMap::new();
    for kind in discovery.discover().await? {
        let group: &str = &kind.gvk.group;
        if kind.preferred && kind.supports(Verb::List) && !is_builtin_api_group(group) {
            groups
                .entry(group.to_owned())
                .or_default()
                .push(CustomKind {
                    kind: kind.gvk.kind.to_string(),
                    plural: kind.plural.clone(),
                });
        }
    }
    Ok(groups
        .into_iter()
        .map(|(group, mut kinds)| {
            kinds.sort();
            kinds.dedup();
            CustomResourceGroup { group, kinds }
        })
        .collect())
}
