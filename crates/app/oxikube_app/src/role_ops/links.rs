//! [`RoleLinks`]: what a Role's views read besides the Role itself.

use std::sync::Arc;

use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::rbac::{
    Binding, BindingKind, RbacKind, Role, RoleId, RoleKind, aggregated_from,
};
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::{ListOptions, ResourceReader};

use crate::session::ClusterSessionManager;

/// Objects per page when listing; the lists are followed to the last page.
const PAGE: u32 = 500;

/// Reads the ClusterRoles and bindings around a role: the sources of an aggregated ClusterRole,
/// and how many bindings name a role.
///
/// Plain reads through the session's reader, made when a view asks (a detail opening, the delete
/// button), not a watch: a Role's detail should not keep every binding of the cluster in memory.
/// Run them off the UI thread (`oxikube_runtime::spawn_kube`).
#[derive(Clone)]
pub struct RoleLinks {
    sessions: ClusterSessionManager,
}

impl RoleLinks {
    /// Reads through `sessions`.
    pub fn new(sessions: ClusterSessionManager) -> Self {
        Self { sessions }
    }

    /// The ClusterRoles `role` (an aggregated ClusterRole) collects from, sorted by name; empty
    /// for a role with no aggregation rule.
    ///
    /// # Errors
    ///
    /// The cluster is not connected, or the list failed.
    pub async fn contributors(&self, cluster: &ClusterId, role: &Role) -> OxiResult<Vec<RoleId>> {
        if role.aggregation.is_none() {
            return Ok(Vec::new());
        }
        let reader = self.reader(cluster)?;
        let all = list_all(&reader, RbacKind::ClusterRole.gvk(), None).await?;
        let roles: Vec<Role> = all
            .iter()
            .filter_map(|resource| Role::from_resource(resource).ok())
            .collect();
        let refs: Vec<&Role> = roles.iter().collect();
        Ok(aggregated_from(role, &refs))
    }

    /// How many RoleBindings and ClusterRoleBindings name `role`.
    ///
    /// # Errors
    ///
    /// The cluster is not connected, or a list failed (a user who may not list bindings gets
    /// `Forbidden`: the delete then goes ahead without the count).
    pub async fn binding_count(&self, cluster: &ClusterId, role: &RoleId) -> OxiResult<u32> {
        let reader = self.reader(cluster)?;
        let mut count = 0usize;
        for kind in [RbacKind::ClusterRoleBinding, RbacKind::RoleBinding] {
            let bindings = list_all(&reader, kind.gvk(), None).await?;
            count += bindings
                .iter()
                .filter_map(|resource| Binding::from_resource(resource).ok())
                .filter(|binding| binds(binding, role))
                .count();
        }
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    fn reader(&self, cluster: &ClusterId) -> OxiResult<Arc<dyn ResourceReader>> {
        self.sessions
            .get(cluster)
            .and_then(|session| session.resources())
            .ok_or_else(|| OxiError::conflict("the cluster is not connected"))
    }
}

/// Whether `binding` grants `role`: a ClusterRole by name from either binding kind, a Role by name
/// from a RoleBinding in the Role's own namespace.
pub fn binds(binding: &Binding, role: &RoleId) -> bool {
    if binding.role_ref.kind != role.kind || binding.role_ref.name != role.name {
        return false;
    }
    match role.kind {
        RoleKind::ClusterRole => true,
        RoleKind::Role => {
            binding.id.kind == BindingKind::RoleBinding && binding.id.namespace == role.namespace
        }
    }
}

pub(crate) async fn list_all(
    reader: &Arc<dyn ResourceReader>,
    gvk: Gvk,
    namespace: Option<&str>,
) -> OxiResult<Vec<Resource>> {
    let mut options = ListOptions {
        limit: Some(PAGE),
        ..ListOptions::default()
    };
    let mut items = Vec::new();
    loop {
        let page = reader.list(&gvk, namespace, &options).await?;
        items.extend(page.items);
        match page.continue_token.filter(|token| !token.is_empty()) {
            Some(token) => options = options.continue_from(token),
            None => return Ok(items),
        }
    }
}
