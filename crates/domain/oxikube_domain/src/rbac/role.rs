//! [`Role`] (a Role or a ClusterRole), [`RoleRef`] and [`RoleId`], and the aggregation of
//! ClusterRoles ([`aggregated_rules`]).

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use super::kind::{RbacError, RbacKind};
use super::normalise::{NormalisedRules, RuleSetBuilder};
use super::policy::PolicyRule;
use super::selector::AggregationRule;
use crate::resource::{Resource, StrMap};

/// Whether a role is a namespaced Role or a ClusterRole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoleKind {
    /// A namespaced `Role`.
    Role,
    /// A `ClusterRole`.
    ClusterRole,
}

impl RoleKind {
    /// `Role` or `ClusterRole`.
    pub fn as_str(self) -> &'static str {
        match self {
            RoleKind::Role => "Role",
            RoleKind::ClusterRole => "ClusterRole",
        }
    }

    /// The matching [`RbacKind`].
    pub fn rbac_kind(self) -> RbacKind {
        match self {
            RoleKind::Role => RbacKind::Role,
            RoleKind::ClusterRole => RbacKind::ClusterRole,
        }
    }
}

/// A binding's `roleRef`: the kind and name of the role it grants. A Role is looked up in the
/// binding's own namespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoleRef {
    /// Role or ClusterRole.
    pub kind: RoleKind,
    /// The role's name.
    pub name: Arc<str>,
}

/// The identity of one role object: kind, namespace (`None` for a ClusterRole) and name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoleId {
    /// Role or ClusterRole.
    pub kind: RoleKind,
    /// The Role's namespace; `None` for a ClusterRole.
    pub namespace: Option<Arc<str>>,
    /// The role's name.
    pub name: Arc<str>,
}

impl RoleId {
    /// A ClusterRole's id.
    pub fn cluster_role(name: &str) -> Self {
        Self {
            kind: RoleKind::ClusterRole,
            namespace: None,
            name: name.into(),
        }
    }

    /// A namespaced Role's id.
    pub fn role(namespace: &str, name: &str) -> Self {
        Self {
            kind: RoleKind::Role,
            namespace: Some(namespace.into()),
            name: name.into(),
        }
    }
}

impl fmt::Display for RoleId {
    /// `ClusterRole view`, `Role demo/reader`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.namespace {
            Some(namespace) => write!(f, "{} {namespace}/{}", self.kind.as_str(), self.name),
            None => write!(f, "{} {}", self.kind.as_str(), self.name),
        }
    }
}

/// A Role or ClusterRole read from its [`Resource`]: identity, labels (what aggregation selects
/// on), the rules as written and, for an aggregated ClusterRole, its aggregation rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    /// Kind, namespace and name.
    pub id: RoleId,
    /// `metadata.labels`.
    pub labels: StrMap,
    /// `rules` as written (for an aggregated ClusterRole, what the control plane last filled in).
    pub rules: Vec<PolicyRule>,
    /// `aggregationRule`, ClusterRoles only.
    pub aggregation: Option<AggregationRule>,
}

impl Role {
    /// Reads a Role or ClusterRole.
    ///
    /// # Errors
    ///
    /// [`RbacError::WrongKind`] for any other kind, [`RbacError::MissingNamespace`] for a Role
    /// without a namespace.
    pub fn from_resource(resource: &Resource) -> Result<Self, RbacError> {
        let kind = match RbacKind::of(&resource.kind) {
            Some(RbacKind::Role) => RoleKind::Role,
            Some(RbacKind::ClusterRole) => RoleKind::ClusterRole,
            _ => {
                return Err(RbacError::WrongKind {
                    expected: "Role or ClusterRole",
                    found: resource.kind.kind.to_string(),
                });
            }
        };
        let namespace = match kind {
            RoleKind::ClusterRole => None,
            RoleKind::Role => Some(resource.meta.namespace.clone().ok_or_else(|| {
                RbacError::MissingNamespace {
                    kind: "Role",
                    name: resource.name().to_owned(),
                }
            })?),
        };
        let aggregation = match kind {
            RoleKind::ClusterRole => resource
                .get("/aggregationRule")
                .filter(|rule| rule.is_object())
                .map(AggregationRule::from_json),
            RoleKind::Role => None,
        };
        Ok(Self {
            id: RoleId {
                kind,
                namespace,
                name: resource.meta.name.clone(),
            },
            labels: resource.meta.labels.clone(),
            rules: PolicyRule::list_from_json(resource.get("/rules")),
            aggregation,
        })
    }

    /// The role's own rules, normalised.
    pub fn normalised(&self) -> NormalisedRules {
        NormalisedRules::from_policy(&self.rules)
    }
}

/// The normalised rules a role grants, aggregation included.
///
/// The control plane copies an aggregated ClusterRole's selected rules into its `rules`, so the
/// live `rules` are authoritative. The selectors are resolved too, against `cluster_roles`
/// (recursively, each ClusterRole once), and the union is returned: on a live cluster that adds
/// nothing, and for an offline fixture or a ClusterRole the controller has not reached yet it
/// fills the gap. A plain Role or ClusterRole is its own rules.
pub fn aggregated_rules<'a>(role: &'a Role, cluster_roles: &[&'a Role]) -> NormalisedRules {
    let mut builder = RuleSetBuilder::default();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut pending = vec![role];
    while let Some(next) = pending.pop() {
        if next.id.kind == RoleKind::ClusterRole && !seen.insert(&next.id.name) {
            continue;
        }
        builder.add_policies(&next.rules);
        let Some(aggregation) = &next.aggregation else {
            continue;
        };
        let selected = cluster_roles.iter().copied().filter(|candidate| {
            candidate.id.kind == RoleKind::ClusterRole
                && candidate.id != next.id
                && aggregation.selects(&candidate.labels)
        });
        pending.extend(selected);
    }
    builder.finish()
}

/// The ClusterRoles an aggregated ClusterRole collects from (directly), sorted by name; empty
/// for a role without an aggregation rule. For the "aggregated from" list of a role's detail.
pub fn aggregated_from(role: &Role, cluster_roles: &[&Role]) -> Vec<RoleId> {
    let Some(aggregation) = &role.aggregation else {
        return Vec::new();
    };
    let mut sources: Vec<RoleId> = cluster_roles
        .iter()
        .filter(|candidate| {
            candidate.id.kind == RoleKind::ClusterRole
                && candidate.id != role.id
                && aggregation.selects(&candidate.labels)
        })
        .map(|candidate| candidate.id.clone())
        .collect();
    sources.sort();
    sources
}
