//! [`PolicyRule`]: one entry of a Role's or ClusterRole's `rules`, as written.

use serde::{Deserialize, Serialize};

use crate::access::AccessRule;
use crate::json::JsonRef;

/// One RBAC rule as the object declares it (`rbac.authorization.k8s.io/v1` `PolicyRule`),
/// before normalisation: wildcards unexpanded, duplicates kept, order as written.
///
/// A rule is either a resource rule (`api_groups` and `resources`, optionally narrowed by
/// `resource_names`) or a non-resource rule (`non_resource_urls`); the API server rejects one
/// that is both, and this type keeps both lists so nothing in a fixture is lost.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyRule {
    /// Verbs; `*` is every verb.
    #[serde(default)]
    pub verbs: Vec<String>,
    /// API groups; `""` is the core group, `*` every group.
    #[serde(default)]
    pub api_groups: Vec<String>,
    /// Resources (`pods`), subresources (`pods/log`), `*` (everything) or `*/scale` (that
    /// subresource of any resource).
    #[serde(default)]
    pub resources: Vec<String>,
    /// When non-empty, only these objects.
    #[serde(default)]
    pub resource_names: Vec<String>,
    /// Non-resource paths (`/healthz`); a trailing `*` is a prefix glob.
    #[serde(default, rename = "nonResourceURLs")]
    pub non_resource_urls: Vec<String>,
}

impl PolicyRule {
    /// A resource rule, for tests and fixtures.
    #[must_use]
    pub fn resource(
        verbs: &[&str],
        api_groups: &[&str],
        resources: &[&str],
        resource_names: &[&str],
    ) -> Self {
        Self {
            verbs: own(verbs),
            api_groups: own(api_groups),
            resources: own(resources),
            resource_names: own(resource_names),
            non_resource_urls: Vec::new(),
        }
    }

    /// A non-resource rule, for tests and fixtures.
    #[must_use]
    pub fn non_resource(verbs: &[&str], urls: &[&str]) -> Self {
        Self {
            verbs: own(verbs),
            non_resource_urls: own(urls),
            ..Self::default()
        }
    }

    /// Reads a rule from its JSON (a `PolicyRule`, or a rules review's `ResourceRule` /
    /// `NonResourceRule`, which use the same field names). Missing lists are empty; entries that
    /// are not strings are skipped.
    pub fn from_json(rule: JsonRef<'_>) -> Self {
        let list = |key: &str| -> Vec<String> {
            rule.get(key)
                .and_then(JsonRef::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(JsonRef::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        Self {
            verbs: list("verbs"),
            api_groups: list("apiGroups"),
            resources: list("resources"),
            resource_names: list("resourceNames"),
            non_resource_urls: list("nonResourceURLs"),
        }
    }

    /// Every rule in the JSON array at `rules` (absent or not an array: none).
    pub fn list_from_json(rules: Option<JsonRef<'_>>) -> Vec<Self> {
        rules
            .and_then(JsonRef::as_array)
            .map(|items| items.iter().map(Self::from_json).collect())
            .unwrap_or_default()
    }

    /// The resource half of the rule as the matcher's [`AccessRule`], unnormalised.
    pub fn to_access_rule(&self) -> AccessRule {
        AccessRule {
            verbs: self.verbs.clone(),
            api_groups: self.api_groups.clone(),
            resources: self.resources.clone(),
            resource_names: self.resource_names.clone(),
        }
    }
}

fn own(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}
