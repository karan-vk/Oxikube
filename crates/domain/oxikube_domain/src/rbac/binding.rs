//! [`Binding`] (a RoleBinding or ClusterRoleBinding), [`BindingRef`] and [`GrantScope`].

use std::fmt;
use std::sync::Arc;

use super::kind::{RbacError, RbacKind};
use super::role::{RoleId, RoleKind, RoleRef};
use super::subject::Subject;
use crate::json::JsonRef;
use crate::resource::Resource;

/// Whether a binding is a RoleBinding or a ClusterRoleBinding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BindingKind {
    /// A namespaced `RoleBinding`.
    RoleBinding,
    /// A `ClusterRoleBinding`.
    ClusterRoleBinding,
}

impl BindingKind {
    /// `RoleBinding` or `ClusterRoleBinding`.
    pub fn as_str(self) -> &'static str {
        match self {
            BindingKind::RoleBinding => "RoleBinding",
            BindingKind::ClusterRoleBinding => "ClusterRoleBinding",
        }
    }

    /// The matching [`RbacKind`].
    pub fn rbac_kind(self) -> RbacKind {
        match self {
            BindingKind::RoleBinding => RbacKind::RoleBinding,
            BindingKind::ClusterRoleBinding => RbacKind::ClusterRoleBinding,
        }
    }
}

/// The identity of one binding object: kind, namespace (`None` for a ClusterRoleBinding) and
/// name. Orders ClusterRoleBindings after RoleBindings, each by namespace and name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BindingRef {
    /// RoleBinding or ClusterRoleBinding.
    pub kind: BindingKind,
    /// The RoleBinding's namespace; `None` for a ClusterRoleBinding.
    pub namespace: Option<Arc<str>>,
    /// The binding's name.
    pub name: Arc<str>,
}

impl fmt::Display for BindingRef {
    /// `ClusterRoleBinding admins`, `RoleBinding demo/readers`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.namespace {
            Some(namespace) => write!(f, "{} {namespace}/{}", self.kind.as_str(), self.name),
            None => write!(f, "{} {}", self.kind.as_str(), self.name),
        }
    }
}

/// Where a binding's grant holds.
///
/// A ClusterRoleBinding grants cluster-wide; a RoleBinding grants only inside its own namespace,
/// whether it names a Role or a ClusterRole. Orders [`GrantScope::Cluster`] first.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GrantScope {
    /// Every namespace, cluster-scoped resources and non-resource URLs.
    Cluster,
    /// Namespaced resources of this namespace only.
    Namespace(Arc<str>),
}

impl GrantScope {
    /// Whether the grant holds for a request in `namespace` (`None`: a cluster-scoped resource,
    /// or every namespace at once, which only a cluster-wide grant covers).
    pub fn covers(&self, namespace: Option<&str>) -> bool {
        match self {
            GrantScope::Cluster => true,
            GrantScope::Namespace(own) => namespace == Some(&**own),
        }
    }
}

/// A RoleBinding or ClusterRoleBinding read from its [`Resource`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// Kind, namespace and name.
    pub id: BindingRef,
    /// The role granted.
    pub role_ref: RoleRef,
    /// Who it is granted to, in the order written; unknown kinds are left out.
    pub subjects: Vec<Subject>,
}

impl Binding {
    /// Reads a RoleBinding or ClusterRoleBinding.
    ///
    /// # Errors
    ///
    /// [`RbacError::WrongKind`] for any other kind, [`RbacError::MissingNamespace`] for a
    /// RoleBinding without a namespace, [`RbacError::InvalidRoleRef`] when `roleRef` is missing,
    /// nameless or names neither a Role nor a ClusterRole.
    pub fn from_resource(resource: &Resource) -> Result<Self, RbacError> {
        let kind = match RbacKind::of(&resource.kind) {
            Some(RbacKind::RoleBinding) => BindingKind::RoleBinding,
            Some(RbacKind::ClusterRoleBinding) => BindingKind::ClusterRoleBinding,
            _ => {
                return Err(RbacError::WrongKind {
                    expected: "RoleBinding or ClusterRoleBinding",
                    found: resource.kind.kind.to_string(),
                });
            }
        };
        let namespace = match kind {
            BindingKind::ClusterRoleBinding => None,
            BindingKind::RoleBinding => Some(resource.meta.namespace.clone().ok_or_else(|| {
                RbacError::MissingNamespace {
                    kind: kind.as_str(),
                    name: resource.name().to_owned(),
                }
            })?),
        };
        let invalid = || RbacError::InvalidRoleRef {
            kind: kind.as_str(),
            name: resource.name().to_owned(),
        };
        let role_ref = resource.get("/roleRef").ok_or_else(invalid)?;
        let text = |key: &str| role_ref.get(key).and_then(JsonRef::as_str);
        let role_kind = match text("kind") {
            Some("Role") => RoleKind::Role,
            Some("ClusterRole") => RoleKind::ClusterRole,
            _ => return Err(invalid()),
        };
        let role_name = text("name")
            .filter(|name| !name.is_empty())
            .ok_or_else(invalid)?;
        let subjects = resource
            .get("/subjects")
            .and_then(JsonRef::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|subject| Subject::from_json(subject, namespace.as_deref()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            id: BindingRef {
                kind,
                namespace,
                name: resource.meta.name.clone(),
            },
            role_ref: RoleRef {
                kind: role_kind,
                name: role_name.into(),
            },
            subjects,
        })
    }

    /// Where the grant holds: the cluster for a ClusterRoleBinding, the binding's namespace for
    /// a RoleBinding.
    pub fn scope(&self) -> GrantScope {
        match &self.id.namespace {
            Some(namespace) => GrantScope::Namespace(namespace.clone()),
            None => GrantScope::Cluster,
        }
    }

    /// The role object the binding grants: a Role in the binding's namespace, or a ClusterRole.
    /// `None` for a ClusterRoleBinding naming a Role, which the API server ignores.
    pub fn role_id(&self) -> Option<RoleId> {
        match (self.role_ref.kind, &self.id.namespace) {
            (RoleKind::ClusterRole, _) => Some(RoleId {
                kind: RoleKind::ClusterRole,
                namespace: None,
                name: self.role_ref.name.clone(),
            }),
            (RoleKind::Role, Some(namespace)) => Some(RoleId {
                kind: RoleKind::Role,
                namespace: Some(namespace.clone()),
                name: self.role_ref.name.clone(),
            }),
            (RoleKind::Role, None) => None,
        }
    }
}
