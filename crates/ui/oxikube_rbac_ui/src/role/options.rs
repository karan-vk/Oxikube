//! [`ResourceOptions`]: what the dialog's resource picker offers, from the cluster's discovery.

use std::collections::BTreeMap;

use oxikube_domain::kinds::{ResourceKind, Verb, VerbSet};

/// Subresources the discovery document does not list, with the verbs they take: `(group,
/// resource, subresource, verbs)`.
const SUBRESOURCES: &[(&str, &str, &str, &[Verb])] = &[
    ("", "pods", "log", &[Verb::Get]),
    ("", "pods", "exec", &[Verb::Get, Verb::Create]),
    ("", "pods", "attach", &[Verb::Get, Verb::Create]),
    ("", "pods", "portforward", &[Verb::Get, Verb::Create]),
    ("", "pods", "eviction", &[Verb::Create]),
    (
        "",
        "pods",
        "status",
        &[Verb::Get, Verb::Update, Verb::Patch],
    ),
    (
        "",
        "pods",
        "ephemeralcontainers",
        &[Verb::Get, Verb::Update, Verb::Patch],
    ),
    (
        "",
        "services",
        "status",
        &[Verb::Get, Verb::Update, Verb::Patch],
    ),
    (
        "",
        "nodes",
        "status",
        &[Verb::Get, Verb::Update, Verb::Patch],
    ),
    ("", "serviceaccounts", "token", &[Verb::Create]),
    (
        "apps",
        "deployments",
        "scale",
        &[Verb::Get, Verb::Update, Verb::Patch],
    ),
    (
        "apps",
        "deployments",
        "status",
        &[Verb::Get, Verb::Update, Verb::Patch],
    ),
    (
        "apps",
        "statefulsets",
        "scale",
        &[Verb::Get, Verb::Update, Verb::Patch],
    ),
    (
        "apps",
        "replicasets",
        "scale",
        &[Verb::Get, Verb::Update, Verb::Patch],
    ),
    (
        "batch",
        "jobs",
        "status",
        &[Verb::Get, Verb::Update, Verb::Patch],
    ),
    (
        "batch",
        "cronjobs",
        "status",
        &[Verb::Get, Verb::Update, Verb::Patch],
    ),
];

/// The verbs a proxy subresource takes.
const PROXY_VERBS: &[Verb] = &[
    Verb::Get,
    Verb::Create,
    Verb::Update,
    Verb::Patch,
    Verb::Delete,
];

/// One resource of the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceOption {
    /// The API group; `""` is the core group.
    pub group: String,
    /// The plural name, `pods`, or with a subresource, `pods/log`.
    pub resource: String,
    /// The verbs the server supports for it.
    pub verbs: VerbSet,
    /// Whether objects live in a namespace (a Role can only grant these).
    pub namespaced: bool,
}

impl ResourceOption {
    /// `group/resource` as the picker lists it (`pods` for the core group).
    pub fn label(&self) -> String {
        if self.group.is_empty() {
            self.resource.clone()
        } else {
            format!("{}/{}", self.group, self.resource)
        }
    }
}

/// The resources a cluster serves, one per group and name, with the subresources the picker
/// adds. Built once from discovery (off the UI thread) and searched on every keystroke.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceOptions {
    options: Vec<ResourceOption>,
}

impl ResourceOptions {
    /// The options of the kinds `discover()` returned: the preferred version of each, the
    /// subresources of [`SUBRESOURCES`] and `proxy` for nodes, pods and services, sorted by
    /// label.
    pub fn from_kinds(kinds: &[ResourceKind]) -> Self {
        let mut by_name: BTreeMap<(String, String), ResourceOption> = BTreeMap::new();
        for kind in kinds.iter().filter(|kind| kind.preferred) {
            by_name.insert(
                (kind.gvk.group.to_string(), kind.plural.clone()),
                ResourceOption {
                    group: kind.gvk.group.to_string(),
                    resource: kind.plural.clone(),
                    verbs: kind.verbs,
                    namespaced: kind.namespaced,
                },
            );
        }
        let served: Vec<(String, String)> = by_name.keys().cloned().collect();
        for (group, resource, sub, verbs) in SUBRESOURCES {
            let key = ((*group).to_owned(), (*resource).to_owned());
            if served.contains(&key) {
                by_name.insert(
                    (key.0.clone(), format!("{resource}/{sub}")),
                    ResourceOption {
                        group: key.0,
                        resource: format!("{resource}/{sub}"),
                        verbs: verb_set(verbs),
                        namespaced: by_name[&(group.to_string(), resource.to_string())].namespaced,
                    },
                );
            }
        }
        for resource in ["nodes", "pods", "services"] {
            let key = (String::new(), resource.to_owned());
            if let Some(parent) = by_name.get(&key).cloned() {
                by_name.insert(
                    (String::new(), format!("{resource}/proxy")),
                    ResourceOption {
                        resource: format!("{resource}/proxy"),
                        verbs: verb_set(PROXY_VERBS),
                        ..parent
                    },
                );
            }
        }
        let mut options: Vec<ResourceOption> = by_name.into_values().collect();
        options.sort_by_key(ResourceOption::label);
        Self { options }
    }

    /// Whether discovery has not answered (or served nothing).
    pub fn is_empty(&self) -> bool {
        self.options.is_empty()
    }

    /// How many resources the picker holds.
    pub fn len(&self) -> usize {
        self.options.len()
    }

    /// The options whose resource or group contains `query` (case-insensitive), the ones that
    /// start with it first, at most `limit`. An empty query lists the first `limit`.
    pub fn suggest(&self, query: &str, limit: usize) -> Vec<&ResourceOption> {
        let query = query.trim().to_lowercase();
        let mut starts: Vec<&ResourceOption> = Vec::new();
        let mut contains: Vec<&ResourceOption> = Vec::new();
        for option in &self.options {
            let label = option.label().to_lowercase();
            if query.is_empty() || option.resource.to_lowercase().starts_with(&query) {
                starts.push(option);
            } else if label.contains(&query) {
                contains.push(option);
            }
        }
        starts.into_iter().chain(contains).take(limit).collect()
    }

    /// The verbs the picked resources take: the union over the ones discovery knows. `None` when
    /// any resource is `*`, a `*/sub` wildcard or not served (a CRD that is not installed, free
    /// text), since then nothing is known to be unsupported. `groups` empty or holding `*` match
    /// every group.
    pub fn supported_verbs(&self, groups: &[String], resources: &[String]) -> Option<VerbSet> {
        if resources.is_empty() {
            return None;
        }
        let any_group = groups.is_empty() || groups.iter().any(|group| group == "*");
        let mut union = VerbSet::default();
        for resource in resources {
            if resource.starts_with('*') {
                return None;
            }
            let found: Vec<&ResourceOption> = self
                .options
                .iter()
                .filter(|option| {
                    option.resource == *resource && (any_group || groups.contains(&option.group))
                })
                .collect();
            if found.is_empty() {
                return None;
            }
            for option in found {
                for verb in option.verbs.iter() {
                    union.insert(verb);
                }
            }
        }
        Some(union)
    }

    /// The API group of the resource `resource` when exactly one served group has it, so picking
    /// `deployments` can fill in `apps`.
    pub fn group_of(&self, resource: &str) -> Option<&str> {
        let mut found = self
            .options
            .iter()
            .filter(|option| option.resource == resource)
            .map(|option| option.group.as_str());
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    /// Whether any option is a namespaced resource of `group`'s `resource`: a Role may grant only
    /// these. `None` when the resource is not known.
    pub fn is_namespaced(&self, group: &str, resource: &str) -> Option<bool> {
        self.options
            .iter()
            .find(|option| option.group == group && option.resource == resource)
            .map(|option| option.namespaced)
    }
}

fn verb_set(verbs: &[Verb]) -> VerbSet {
    let mut set = VerbSet::default();
    for verb in verbs {
        set.insert(*verb);
    }
    set
}
