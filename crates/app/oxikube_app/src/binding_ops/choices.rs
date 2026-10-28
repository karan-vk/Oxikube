//! [`BindingChoices`]: what the binding dialog offers, and [`role_of`], the role a binding names.

use std::collections::BTreeSet;
use std::sync::Arc;

use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::rbac::{Binding, RbacKind, Role, RoleId, RoleKind, Subject};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::ResourceReader;

use crate::role_ops::{list_all, list_all_metadata};
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
    /// The five lists run at once, not one after another, and the Roles, ClusterRoles and
    /// ServiceAccounts are read as metadata only (the pickers need their names); only the
    /// bindings are read whole, for the subject names they use.
    ///
    /// # Errors
    ///
    /// The cluster is not connected.
    pub async fn read(sessions: &ClusterSessionManager, cluster: &ClusterId) -> OxiResult<Self> {
        let reader = sessions
            .get(cluster)
            .and_then(|session| session.resources())
            .ok_or_else(|| OxiError::conflict("the cluster is not connected"))?;
        Ok(Self::read_from(&reader).await)
    }

    /// Reads the lists through `reader`, all at once.
    pub(crate) async fn read_from(reader: &Arc<dyn ResourceReader>) -> Self {
        let (roles, cluster_roles, accounts, cluster_bindings, bindings) = futures::join!(
            list_all_metadata(reader, RbacKind::Role.gvk(), None),
            list_all_metadata(reader, RbacKind::ClusterRole.gvk(), None),
            list_all_metadata(reader, Gvk::new("", "v1", "ServiceAccount"), None),
            list_all(reader, RbacKind::ClusterRoleBinding.gvk(), None),
            list_all(reader, RbacKind::RoleBinding.gvk(), None),
        );
        let mut choices = Self::default();
        match roles {
            Ok(items) => {
                let mut roles: Vec<RoleId> = items
                    .into_iter()
                    .filter_map(|meta| {
                        Some(RoleId {
                            kind: RoleKind::Role,
                            namespace: Some(meta.namespace?),
                            name: meta.name,
                        })
                    })
                    .collect();
                roles.sort();
                choices.roles = roles;
            }
            Err(error) => choices.note("Roles", &error),
        }
        match cluster_roles {
            Ok(items) => {
                let mut names: Vec<Arc<str>> = items.into_iter().map(|meta| meta.name).collect();
                names.sort();
                choices.cluster_roles = names;
            }
            Err(error) => choices.note("ClusterRoles", &error),
        }
        match accounts {
            Ok(items) => {
                let mut accounts: Vec<(Arc<str>, Arc<str>)> = items
                    .into_iter()
                    .filter_map(|meta| Some((meta.namespace?, meta.name)))
                    .collect();
                accounts.sort();
                choices.service_accounts = accounts;
            }
            Err(error) => choices.note("ServiceAccounts", &error),
        }
        let (mut users, mut groups) = (BTreeSet::new(), BTreeSet::new());
        for (kind, listed) in [
            (RbacKind::ClusterRoleBinding, cluster_bindings),
            (RbacKind::RoleBinding, bindings),
        ] {
            match listed {
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
        choices
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
