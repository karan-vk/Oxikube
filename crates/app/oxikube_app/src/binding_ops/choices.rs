//! [`BindingChoices`]: what the binding dialog offers, and [`role_of`], the role a binding names.

use std::collections::BTreeSet;
use std::sync::Arc;

use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::rbac::{Binding, RbacKind, Role, RoleId, RoleKind, Subject};
use oxikube_domain::{OxiError, OxiResult};

use crate::role_ops::list_all;
use crate::session::ClusterSessionManager;

/// What the dialog's pickers offer, read from the cluster.
///
/// Every list is read on its own: a user who may not list ServiceAccounts still gets the roles.
/// What could not be read is named in [`notes`](Self::notes) and its picker falls back to typing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingChoices {
    /// The Roles of every namespace, sorted by namespace and name.
    pub roles: Vec<RoleId>,
    /// The ClusterRoles, sorted.
    pub cluster_roles: Vec<Arc<str>>,
    /// The ServiceAccounts of every namespace, as `(namespace, name)`, sorted.
    pub service_accounts: Vec<(Arc<str>, Arc<str>)>,
    /// The user names existing bindings already use, sorted (the autocomplete).
    pub users: Vec<Arc<str>>,
    /// The group names existing bindings already use, sorted (the autocomplete).
    pub groups: Vec<Arc<str>>,
    /// What could not be read, one line each.
    pub notes: Vec<String>,
}

impl BindingChoices {
    /// Reads the lists through `sessions`. Run it off the UI thread (`spawn_kube`).
    ///
    /// # Errors
    ///
    /// The cluster is not connected.
    pub async fn read(sessions: &ClusterSessionManager, cluster: &ClusterId) -> OxiResult<Self> {
        let reader = sessions
            .get(cluster)
            .and_then(|session| session.resources())
            .ok_or_else(|| OxiError::conflict("the cluster is not connected"))?;
        let mut choices = Self::default();
        match list_all(&reader, RbacKind::Role.gvk(), None).await {
            Ok(items) => {
                let mut roles: Vec<RoleId> = items
                    .iter()
                    .filter_map(|item| Role::from_resource(item).ok())
                    .map(|role| role.id)
                    .collect();
                roles.sort();
                choices.roles = roles;
            }
            Err(error) => choices.note("Roles", &error),
        }
        match list_all(&reader, RbacKind::ClusterRole.gvk(), None).await {
            Ok(items) => {
                let mut names: Vec<Arc<str>> =
                    items.iter().map(|item| item.meta.name.clone()).collect();
                names.sort();
                choices.cluster_roles = names;
            }
            Err(error) => choices.note("ClusterRoles", &error),
        }
        match list_all(&reader, Gvk::new("", "v1", "ServiceAccount"), None).await {
            Ok(items) => {
                let mut accounts: Vec<(Arc<str>, Arc<str>)> = items
                    .iter()
                    .filter_map(|item| Some((item.meta.namespace.clone()?, item.meta.name.clone())))
                    .collect();
                accounts.sort();
                choices.service_accounts = accounts;
            }
            Err(error) => choices.note("ServiceAccounts", &error),
        }
        let (mut users, mut groups) = (BTreeSet::new(), BTreeSet::new());
        for kind in [RbacKind::ClusterRoleBinding, RbacKind::RoleBinding] {
            match list_all(&reader, kind.gvk(), None).await {
                Ok(items) => {
                    for subject in items
                        .iter()
                        .filter_map(|item| Binding::from_resource(item).ok())
                        .flat_map(|binding| binding.subjects)
                    {
                        match subject {
                            Subject::User(name) => {
                                users.insert(name);
                            }
                            Subject::Group(name) => {
                                groups.insert(name);
                            }
                            Subject::ServiceAccount { .. } => {}
                        }
                    }
                }
                Err(error) => choices.note(kind.as_str(), &error),
            }
        }
        choices.users = users.into_iter().collect();
        choices.groups = groups.into_iter().collect();
        Ok(choices)
    }

    fn note(&mut self, what: &str, error: &OxiError) {
        self.notes.push(format!(
            "{what} could not be listed ({}): type the name instead",
            error.message()
        ));
    }

    /// The roles a binding in `namespace` (`None`: a ClusterRoleBinding) may name: a ClusterRoleBinding
    /// only ClusterRoles, a RoleBinding the Roles of its own namespace and every ClusterRole.
    pub fn offered_roles(&self, namespace: Option<&str>) -> Vec<(RoleKind, Arc<str>)> {
        let mut offered = Vec::new();
        if let Some(namespace) = namespace {
            offered.extend(
                self.roles
                    .iter()
                    .filter(|role| role.namespace.as_deref() == Some(namespace))
                    .map(|role| (RoleKind::Role, role.name.clone())),
            );
        }
        offered.extend(
            self.cluster_roles
                .iter()
                .map(|name| (RoleKind::ClusterRole, name.clone())),
        );
        offered
    }
}

/// The role `binding` names: read fresh, `None` when it no longer exists.
///
/// # Errors
///
/// The cluster is not connected, or the read failed (`Forbidden` for a user who may not read
/// roles).
pub async fn role_of(
    sessions: &ClusterSessionManager,
    cluster: &ClusterId,
    binding: &Binding,
) -> OxiResult<Option<Role>> {
    let reader = sessions
        .get(cluster)
        .and_then(|session| session.resources())
        .ok_or_else(|| OxiError::conflict("the cluster is not connected"))?;
    let Some(id) = binding.role_id() else {
        return Ok(None);
    };
    let resource = reader
        .get_opt(
            &id.kind.rbac_kind().gvk(),
            id.namespace.as_deref(),
            &id.name,
        )
        .await?;
    Ok(resource.and_then(|resource| Role::from_resource(&resource).ok()))
}
