//! What the ServiceAccount panel shows, as plain data: the object's own fields and the roles bound
//! to it.

use std::sync::Arc;

use oxikube_app::access::{Grant, PolicyFeed, PolicyStatus, SubjectIndex};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::rbac::{
    BindingKind, BindingRef, GrantScope, RbacKind, RoleKind, RoleRef, Subject,
};

/// `automountServiceAccountToken`: whether Pods running as the account get its token mounted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Automount {
    /// Not set on the account: Pods mount the token unless their own spec says otherwise.
    Unset,
    /// `true`.
    Enabled,
    /// `false`: Pods do not mount the token unless their own spec says otherwise.
    Disabled,
}

impl Automount {
    /// A short word for the flag.
    pub fn label(self) -> &'static str {
        match self {
            Automount::Unset => "not set",
            Automount::Enabled => "true",
            Automount::Disabled => "false",
        }
    }

    /// What it means for a Pod, including the default when unset.
    pub fn explanation(self) -> &'static str {
        match self {
            Automount::Unset => {
                "Not set: the Kubernetes default is true, so Pods mount the token unless their spec opts out."
            }
            Automount::Enabled => "Pods mount the token unless their own spec opts out.",
            Automount::Disabled => "Pods do not mount the token unless their own spec opts in.",
        }
    }
}

/// One role bound to the account: the binding that grants it, where it holds and how the binding
/// reached the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundRole {
    /// The binding.
    pub binding: BindingRef,
    /// The role the binding names.
    pub role: RoleRef,
    /// Where the grant holds: the whole cluster or one namespace.
    pub scope: GrantScope,
    /// The group the binding names when it reached the account through one
    /// (`system:serviceaccounts:<ns>`, `system:authenticated`); `None` when it names the account.
    pub via_group: Option<Arc<str>>,
    /// Whether the role exists. A binding to a missing role grants nothing.
    pub role_found: bool,
}

impl BoundRole {
    fn of(grant: Grant) -> Self {
        let via_group = match grant.via {
            Subject::Group(group) => Some(group),
            Subject::User(_) | Subject::ServiceAccount { .. } => None,
        };
        Self {
            binding: grant.binding,
            role: grant.role,
            scope: grant.scope,
            via_group,
            role_found: grant.role_found,
        }
    }

    /// The role object to open: a Role in the binding's namespace, or a ClusterRole.
    pub fn role_target(&self, cluster: &ClusterId) -> ResourceRef {
        match self.role.kind {
            RoleKind::Role => ResourceRef::new(
                cluster.clone(),
                RbacKind::Role.gvk(),
                self.binding.namespace.clone(),
                self.role.name.clone(),
            ),
            RoleKind::ClusterRole => ResourceRef::cluster_scoped(
                cluster.clone(),
                RbacKind::ClusterRole.gvk(),
                self.role.name.clone(),
            ),
        }
    }

    /// The binding object to open.
    pub fn binding_target(&self, cluster: &ClusterId) -> ResourceRef {
        let kind = match self.binding.kind {
            BindingKind::RoleBinding => RbacKind::RoleBinding,
            BindingKind::ClusterRoleBinding => RbacKind::ClusterRoleBinding,
        };
        ResourceRef::new(
            cluster.clone(),
            kind.gvk(),
            self.binding.namespace.clone(),
            self.binding.name.clone(),
        )
    }

    /// Where the grant holds, in words: `cluster-wide` or `namespace shop`.
    pub fn scope_label(&self) -> String {
        match &self.scope {
            GrantScope::Cluster => "cluster-wide".to_owned(),
            GrantScope::Namespace(namespace) => format!("namespace {namespace}"),
        }
    }
}

/// The roles bound to the account, or why that is not known (yet).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BoundRoles {
    /// The Role and binding feeds are still being listed: "loading", never "none".
    #[default]
    Loading,
    /// The roles, sorted by binding. `missing` names the RBAC kinds the user may not list: the
    /// rows are then what the readable kinds show, and the panel says so.
    Known {
        /// The bound roles.
        roles: Vec<BoundRole>,
        /// The kinds that could not be listed, and why.
        missing: Vec<(RbacKind, String)>,
    },
}

impl BoundRoles {
    /// The rows, `0` while loading.
    pub fn len(&self) -> usize {
        match self {
            BoundRoles::Loading => 0,
            BoundRoles::Known { roles, .. } => roles.len(),
        }
    }

    /// Whether there are no rows.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The roles bound to `subject` as the feed knows them now. Cheap enough for a background
    /// task; never call it from the render path.
    pub fn read(feed: &PolicyFeed, subject: &Subject) -> Self {
        feed.read(|index| Self::of_index(index, feed.status(), subject))
    }

    /// The roles `index` binds to `subject`, given what is `status` of the feeds behind it.
    pub fn of_index(index: &SubjectIndex, status: PolicyStatus, subject: &Subject) -> Self {
        let missing = match status {
            PolicyStatus::Warming => return BoundRoles::Loading,
            PolicyStatus::Ready => Vec::new(),
            PolicyStatus::Partial(missing) => missing,
        };
        let roles = index
            .grants_for(subject)
            .into_iter()
            .map(BoundRole::of)
            .collect();
        BoundRoles::Known { roles, missing }
    }
}

/// A ServiceAccount's own fields, read from its JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceAccountFacts {
    /// The account's namespace.
    pub namespace: Arc<str>,
    /// The account's name.
    pub name: Arc<str>,
    /// `secrets[].name`: the token Secrets the account lists.
    pub secrets: Vec<Arc<str>>,
    /// `imagePullSecrets[].name`.
    pub image_pull_secrets: Vec<Arc<str>>,
    /// `automountServiceAccountToken`.
    pub automount: Automount,
}

impl ServiceAccountFacts {
    /// Reads `resource`; missing fields read as none / unset.
    pub fn from_resource(resource: &Resource) -> Self {
        Self {
            namespace: resource.namespace().unwrap_or_default().into(),
            name: resource.name().into(),
            secrets: names(resource, "/secrets"),
            image_pull_secrets: names(resource, "/imagePullSecrets"),
            automount: match resource.get_bool("/automountServiceAccountToken") {
                None => Automount::Unset,
                Some(true) => Automount::Enabled,
                Some(false) => Automount::Disabled,
            },
        }
    }

    /// The subject the account authenticates as, the key into the `SubjectIndex`.
    pub fn subject(&self) -> Subject {
        Subject::service_account(&self.namespace, &self.name)
    }
}

/// The `name`s of the objects in the array at `pointer`.
fn names(resource: &Resource, pointer: &str) -> Vec<Arc<str>> {
    resource
        .get(pointer)
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("name")?.as_str().map(Arc::from))
                .collect()
        })
        .unwrap_or_default()
}
