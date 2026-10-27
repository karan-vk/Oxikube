//! [`RbacKind`]: the four RBAC kinds, and [`RbacError`], why an object is not one of them.

use crate::ids::Gvk;

/// The API group of every RBAC kind.
pub const RBAC_GROUP: &str = "rbac.authorization.k8s.io";

/// One of the four kinds RBAC policy is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RbacKind {
    /// A namespaced set of rules.
    Role,
    /// A cluster-wide set of rules, possibly aggregated from other ClusterRoles.
    ClusterRole,
    /// Grants a Role or ClusterRole inside one namespace.
    RoleBinding,
    /// Grants a ClusterRole across the cluster.
    ClusterRoleBinding,
}

impl RbacKind {
    /// Every kind, roles first.
    pub const ALL: [RbacKind; 4] = [
        RbacKind::Role,
        RbacKind::ClusterRole,
        RbacKind::RoleBinding,
        RbacKind::ClusterRoleBinding,
    ];

    /// The kind's name (`ClusterRoleBinding`).
    pub fn as_str(self) -> &'static str {
        match self {
            RbacKind::Role => "Role",
            RbacKind::ClusterRole => "ClusterRole",
            RbacKind::RoleBinding => "RoleBinding",
            RbacKind::ClusterRoleBinding => "ClusterRoleBinding",
        }
    }

    /// The kind of a [`Gvk`], whatever its version; `None` outside `rbac.authorization.k8s.io`.
    pub fn of(gvk: &Gvk) -> Option<Self> {
        if &*gvk.group != RBAC_GROUP {
            return None;
        }
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == &*gvk.kind)
    }

    /// The `rbac.authorization.k8s.io/v1` [`Gvk`] of the kind.
    pub fn gvk(self) -> Gvk {
        Gvk::new(RBAC_GROUP, "v1", self.as_str())
    }

    /// Whether objects of the kind live in a namespace.
    pub fn is_namespaced(self) -> bool {
        matches!(self, RbacKind::Role | RbacKind::RoleBinding)
    }
}

/// Why a [`Resource`](crate::Resource) could not be read as an RBAC object.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RbacError {
    /// The object is not the RBAC kind that was asked for.
    #[error("{found} is not a {expected}")]
    WrongKind {
        /// The kind that was asked for.
        expected: &'static str,
        /// The object's kind.
        found: String,
    },
    /// A namespaced kind has no `metadata.namespace`.
    #[error("{kind} {name} has no namespace")]
    MissingNamespace {
        /// The object's kind.
        kind: &'static str,
        /// The object's name.
        name: String,
    },
    /// A binding has no usable `roleRef` (absent, nameless, or not a Role or ClusterRole).
    #[error("{kind} {name} has no valid roleRef")]
    InvalidRoleRef {
        /// The binding's kind.
        kind: &'static str,
        /// The binding's name.
        name: String,
    },
}
