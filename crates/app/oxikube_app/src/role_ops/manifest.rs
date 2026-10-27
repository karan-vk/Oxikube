//! [`RoleManifest`]: a Role or ClusterRole as the commands carry it.

use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::rbac::{Role, RoleKind};
use oxikube_domain::{OxiError, OxiResult, Resource};
use serde_json::Value;

/// One Role or ClusterRole read from the JSON text of `role::Create` or `role::Edit`.
#[derive(Debug, Clone)]
pub struct RoleManifest {
    /// The object as the API takes it.
    pub body: Value,
    /// Role or ClusterRole.
    pub kind: RoleKind,
    /// The Role's namespace; `None` for a ClusterRole.
    pub namespace: Option<String>,
    /// The object's name.
    pub name: String,
    /// How many rules it holds.
    pub rules: usize,
    /// `metadata.resourceVersion`, when the manifest carries one.
    pub resource_version: Option<String>,
}

impl RoleManifest {
    /// Reads `text` as one Role or ClusterRole.
    ///
    /// # Errors
    ///
    /// A `Validation` error when the text is not JSON, is any other kind, names no object, a Role
    /// has no namespace, or a ClusterRole has one.
    pub fn parse(text: &str) -> OxiResult<Self> {
        let body: Value = serde_json::from_str(text).map_err(|error| {
            OxiError::validation(format!("the role is not valid JSON: {error}"))
        })?;
        let resource = Resource::from_json(body.clone())
            .map_err(|error| OxiError::validation(format!("not a Kubernetes object: {error}")))?;
        let role = Role::from_resource(&resource)
            .map_err(|error| OxiError::validation(error.to_string()))?;
        let kind = role.id.kind;
        if kind == RoleKind::ClusterRole && resource.meta.namespace.is_some() {
            return Err(OxiError::validation("a ClusterRole has no namespace"));
        }
        Ok(Self {
            body,
            kind,
            namespace: role.id.namespace.as_deref().map(str::to_owned),
            name: role.id.name.to_string(),
            rules: role.rules.len(),
            resource_version: resource.meta.resource_version.as_deref().map(str::to_owned),
        })
    }

    /// The `rbac.authorization.k8s.io/v1` kind.
    pub fn gvk(&self) -> Gvk {
        self.kind.rbac_kind().gvk()
    }

    /// The object, in `cluster`.
    pub fn target(&self, cluster: &ClusterId) -> ResourceRef {
        ResourceRef::new(
            cluster.clone(),
            self.gvk(),
            self.namespace.as_deref().map(Into::into),
            self.name.as_str(),
        )
    }
}
