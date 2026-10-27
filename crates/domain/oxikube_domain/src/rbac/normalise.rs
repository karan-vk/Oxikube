//! The rule normaliser: [`PolicyRule`]s in, [`NormalisedRules`] out.
//!
//! # What normalising does
//!
//! 1. **Break down** each rule into one entry per (API group, resource, resource name) and one
//!    per non-resource URL, as `kubectl auth can-i --list` prints them. `pods/log` stays one
//!    resource (the matcher splits the subresource); `resourceNames` stay a restriction on the
//!    entry; `nonResourceURLs` are kept apart, a trailing `*` being a prefix glob.
//! 2. **Merge** entries with the same key: their verbs are unioned, and a `*` verb absorbs the
//!    rest.
//! 3. **Prune** what a wider entry already grants: a verb of `deployments` in `apps` goes when
//!    `*` in `apps` (or `*` in `*`) has it too; a verb on a named object goes when the unnamed
//!    entry has it; `/metrics/x` goes under `/metrics/*`.
//! 4. **Sort**: resource entries by group, resource, name; then URLs; verbs sorted.
//!
//! Wildcards are kept as wildcards rather than expanded to the resources discovery lists: the
//! matcher ([`AccessRule::level_for`]) treats `*` and `*/sub` the way the API server does, and a
//! wildcard also covers resources installed later. The result answers every question exactly as
//! the input rules do (property-tested), and equal policies produce equal sets however they were
//! split across rules, roles or bindings.

use std::collections::{BTreeMap, BTreeSet};

use super::policy::PolicyRule;
use crate::access::{Access, AccessRule, AccessRules, non_resource_url_matches, resource_matches};

const ALL: &str = "*";

/// One normalised grant: a resource rule for exactly one API group, one resource and at most
/// one resource name, or a rule for one non-resource URL. Verbs are sorted, de-duplicated, and
/// just `*` when every verb is granted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalisedRule {
    /// A resource rule, in the matcher's own type.
    Resource(AccessRule),
    /// A non-resource URL rule.
    NonResource(NonResourceRule),
}

/// Verbs on one non-resource URL (`/healthz`, `/metrics/*`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonResourceRule {
    /// The path; a trailing `*` is a prefix glob, `*` alone every path.
    pub url: String,
    /// Sorted verbs (`get`, `post`, ...); `*` is every verb.
    pub verbs: Vec<String>,
}

impl NonResourceRule {
    /// Whether the rule allows `verb` on `path`.
    pub fn allows(&self, verb: &str, path: &str) -> bool {
        non_resource_url_matches(&self.url, path)
            && self.verbs.iter().any(|v| v == ALL || v == verb)
    }
}

/// A normalised, sorted, de-duplicated rule set. See the [module docs](self).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NormalisedRules {
    rules: Vec<NormalisedRule>,
}

impl NormalisedRules {
    /// Normalises `rules`.
    pub fn from_policy(rules: &[PolicyRule]) -> Self {
        let mut builder = RuleSetBuilder::default();
        builder.add_policies(rules);
        builder.finish()
    }

    /// The normalised union of several sets (what a subject holds through several bindings).
    pub fn union<'a>(sets: impl IntoIterator<Item = &'a NormalisedRules>) -> Self {
        let mut builder = RuleSetBuilder::default();
        for set in sets {
            builder.add_normalised(set);
        }
        builder.finish()
    }

    /// The rules, resource rules first.
    pub fn rules(&self) -> &[NormalisedRule] {
        &self.rules
    }

    /// Number of rules.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Whether the set grants nothing.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The resource rules.
    pub fn resource_rules(&self) -> impl Iterator<Item = &AccessRule> {
        self.rules.iter().filter_map(|rule| match rule {
            NormalisedRule::Resource(rule) => Some(rule),
            NormalisedRule::NonResource(_) => None,
        })
    }

    /// The non-resource URL rules.
    pub fn non_resource_rules(&self) -> impl Iterator<Item = &NonResourceRule> {
        self.rules.iter().filter_map(|rule| match rule {
            NormalisedRule::NonResource(rule) => Some(rule),
            NormalisedRule::Resource(_) => None,
        })
    }

    /// The same set without its non-resource URL rules: what a namespaced grant (a RoleBinding)
    /// can give, since URLs are never namespaced.
    #[must_use]
    pub fn resource_only(&self) -> Self {
        Self {
            rules: self
                .rules
                .iter()
                .filter(|rule| matches!(rule, NormalisedRule::Resource(_)))
                .cloned()
                .collect(),
        }
    }

    /// The resource rules as a complete [`AccessRules`] (nothing unknown, nothing partial), for
    /// everything that already reads a rules review.
    pub fn to_access_rules(&self) -> AccessRules {
        AccessRules {
            rules: self.resource_rules().cloned().collect(),
            partial: false,
            namespaces_unseen: false,
        }
    }

    /// What the set says about `verb` on `group`/`resource` (optionally the object `name`):
    /// [`Access::Granted`], [`Access::Restricted`] or [`Access::Denied`], through the same
    /// matcher as [`AccessRules::level_for`].
    pub fn level(&self, verb: &str, group: &str, resource: &str, name: Option<&str>) -> Access {
        self.resource_rules()
            .filter_map(|rule| rule.level_for(verb, group, resource, name))
            .max()
            .unwrap_or(Access::Denied)
    }

    /// Whether the set allows `verb` on the non-resource `path`.
    pub fn allows_url(&self, verb: &str, path: &str) -> bool {
        self.non_resource_rules()
            .any(|rule| rule.allows(verb, path))
    }
}

/// One resource entry's key in the breakdown.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ResourceKey {
    api_group: String,
    resource: String,
    name: Option<String>,
}

impl ResourceKey {
    /// Whether every request this key matches, `wider` matches too.
    fn is_covered_by(&self, wider: &ResourceKey) -> bool {
        (wider.api_group == ALL || wider.api_group == self.api_group)
            && resource_matches(&wider.resource, &self.resource)
            && (wider.name.is_none() || wider.name == self.name)
    }
}

/// Collects rules, then normalises them all at once ([`finish`](Self::finish)).
#[derive(Debug, Default)]
pub(super) struct RuleSetBuilder {
    resources: BTreeMap<ResourceKey, BTreeSet<String>>,
    urls: BTreeMap<String, BTreeSet<String>>,
}

impl RuleSetBuilder {
    /// Breaks `rules` down into the builder.
    pub(super) fn add_policies(&mut self, rules: &[PolicyRule]) {
        for rule in rules {
            self.add_policy(rule);
        }
    }

    fn add_policy(&mut self, rule: &PolicyRule) {
        if rule.verbs.is_empty() {
            return;
        }
        for api_group in &rule.api_groups {
            for resource in &rule.resources {
                let names: Vec<Option<&String>> = if rule.resource_names.is_empty() {
                    vec![None]
                } else {
                    rule.resource_names.iter().map(Some).collect()
                };
                for name in names {
                    let key = ResourceKey {
                        api_group: api_group.clone(),
                        resource: resource.clone(),
                        name: name.cloned(),
                    };
                    self.resources
                        .entry(key)
                        .or_default()
                        .extend(rule.verbs.iter().cloned());
                }
            }
        }
        for url in &rule.non_resource_urls {
            self.urls
                .entry(url.clone())
                .or_default()
                .extend(rule.verbs.iter().cloned());
        }
    }

    /// Adds an already normalised set (a union of sets).
    fn add_normalised(&mut self, rules: &NormalisedRules) {
        for rule in rules.rules() {
            match rule {
                NormalisedRule::Resource(rule) => self.add_policy(&PolicyRule {
                    verbs: rule.verbs.clone(),
                    api_groups: rule.api_groups.clone(),
                    resources: rule.resources.clone(),
                    resource_names: rule.resource_names.clone(),
                    non_resource_urls: Vec::new(),
                }),
                NormalisedRule::NonResource(rule) => {
                    self.urls
                        .entry(rule.url.clone())
                        .or_default()
                        .extend(rule.verbs.iter().cloned());
                }
            }
        }
    }

    /// Merges, prunes and sorts (steps 2 to 4 of the [module docs](self)).
    pub(super) fn finish(self) -> NormalisedRules {
        let resources = prune(collapse(self.resources), ResourceKey::is_covered_by);
        let urls = prune(collapse(self.urls), |url: &String, wider: &String| {
            non_resource_url_matches(wider, url)
        });
        let mut rules = Vec::with_capacity(resources.len() + urls.len());
        for (key, verbs) in resources {
            rules.push(NormalisedRule::Resource(AccessRule {
                verbs,
                api_groups: vec![key.api_group],
                resources: vec![key.resource],
                resource_names: key.name.into_iter().collect(),
            }));
        }
        for (url, verbs) in urls {
            rules.push(NormalisedRule::NonResource(NonResourceRule { url, verbs }));
        }
        NormalisedRules { rules }
    }
}

/// A verb set holding `*` becomes just `*`.
fn collapse<K: Ord>(entries: BTreeMap<K, BTreeSet<String>>) -> BTreeMap<K, BTreeSet<String>> {
    entries
        .into_iter()
        .map(|(key, verbs)| {
            if verbs.contains(ALL) {
                (key, BTreeSet::from([ALL.to_owned()]))
            } else {
                (key, verbs)
            }
        })
        .collect()
}

/// Drops each verb another (different, wider) key already grants, then the keys left with none.
/// Coverage is a partial order (transitive), so checking against the unpruned input is enough.
fn prune<K: Ord>(
    entries: BTreeMap<K, BTreeSet<String>>,
    is_covered_by: impl Fn(&K, &K) -> bool,
) -> Vec<(K, Vec<String>)> {
    let pruned: Vec<Vec<String>> = entries
        .iter()
        .map(|(key, verbs)| {
            let wider: Vec<&BTreeSet<String>> = entries
                .iter()
                .filter(|(other, _)| *other != key && is_covered_by(key, other))
                .map(|(_, verbs)| verbs)
                .collect();
            verbs
                .iter()
                .filter(|verb| {
                    !wider
                        .iter()
                        .any(|granted| granted.contains(ALL) || granted.contains(*verb))
                })
                .cloned()
                .collect()
        })
        .collect();
    entries
        .into_keys()
        .zip(pruned)
        .filter(|(_, verbs)| !verbs.is_empty())
        .collect()
}
