//! [`BindingManifest`]: a RoleBinding or ClusterRoleBinding as the commands carry it.

use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::rbac::{
    Binding, BindingKind, Escalation, RBAC_GROUP, RoleKind, RoleRef, Subject,
};
use oxikube_domain::{OxiError, OxiResult, Resource};
use serde_json::Value;

/// One RoleBinding or ClusterRoleBinding read from the JSON text of `binding::Create` or
/// `binding::Edit`.
#[derive(Debug, Clone)]
pub struct BindingManifest {
    /// The object as the API takes it.
    pub body: Value,
    /// RoleBinding or ClusterRoleBinding.
    pub kind: BindingKind,
    /// The RoleBinding's namespace; `None` for a ClusterRoleBinding.
    pub namespace: Option<String>,
    /// The object's name.
    pub name: String,
    /// The role it grants.
    pub role_ref: RoleRef,
    /// Who it grants to.
    pub subjects: Vec<Subject>,
    /// `metadata.resourceVersion`, when the manifest carries one.
    pub resource_version: Option<String>,
}

impl BindingManifest {
    /// Reads `text` as one RoleBinding or ClusterRoleBinding.
    ///
    /// # Errors
    ///
    /// A `Validation` error when the text is not JSON, is any other kind, names no object, a
    /// RoleBinding has no namespace, a ClusterRoleBinding has one or names a Role, the `roleRef`
    /// has a wrong `apiGroup`, or a subject has a wrong kind, a wrong `apiGroup` (User and Group
    /// take `rbac.authorization.k8s.io`, a ServiceAccount none) or no name (a ServiceAccount
    /// needs a namespace too).
    pub fn parse(text: &str) -> OxiResult<Self> {
        let body: Value = serde_json::from_str(text).map_err(|error| {
            OxiError::validation(format!("the binding is not valid JSON: {error}"))
        })?;
        let resource = Resource::from_json(body.clone())
            .map_err(|error| OxiError::validation(format!("not a Kubernetes object: {error}")))?;
        let binding = Binding::from_resource(&resource)
            .map_err(|error| OxiError::validation(error.to_string()))?;
        let kind = binding.id.kind;
        if kind == BindingKind::ClusterRoleBinding {
            if resource.meta.namespace.is_some() {
                return Err(OxiError::validation(
                    "a ClusterRoleBinding has no namespace",
                ));
            }
            if binding.role_ref.kind != RoleKind::ClusterRole {
                return Err(OxiError::validation(
                    "a ClusterRoleBinding can only name a ClusterRole",
                ));
            }
        }
        check_group(body.pointer("/roleRef/apiGroup"), "roleRef")?;
        check_subjects(body.get("subjects"))?;
        Ok(Self {
            body,
            kind,
            namespace: binding.id.namespace.as_deref().map(str::to_owned),
            name: binding.id.name.to_string(),
            role_ref: binding.role_ref,
            subjects: binding.subjects,
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

    /// `ns/name`, or `name` for a ClusterRoleBinding.
    pub fn label(&self) -> String {
        match &self.namespace {
            Some(namespace) => format!("{namespace}/{}", self.name),
            None => self.name.clone(),
        }
    }

    /// `ClusterRole/view`, as the role column shows it.
    pub fn role_label(&self) -> String {
        format!("{}/{}", self.role_ref.kind.as_str(), self.role_ref.name)
    }

    /// The reasons this grant deserves a louder warning.
    pub fn escalations(&self) -> Vec<Escalation> {
        Escalation::of(&self.role_ref, &self.subjects)
    }

    /// The object as the API takes a new one: without the fields the server fills in.
    pub fn for_create(&self) -> Value {
        let mut body = self.body.clone();
        if let Some(object) = body.as_object_mut() {
            object.remove("status");
            if let Some(Value::Object(metadata)) = object.get_mut("metadata") {
                for key in [
                    "resourceVersion",
                    "uid",
                    "creationTimestamp",
                    "managedFields",
                    "selfLink",
                    "generation",
                    "deletionTimestamp",
                ] {
                    metadata.remove(key);
                }
            }
        }
        body
    }
}

/// An absent `apiGroup` is filled in by the API; a present one must be the RBAC group.
fn check_group(group: Option<&Value>, what: &str) -> OxiResult<()> {
    match group.and_then(Value::as_str) {
        None | Some(RBAC_GROUP) => Ok(()),
        Some(other) => Err(OxiError::validation(format!(
            "{what}.apiGroup is {other:?}; it must be {RBAC_GROUP}"
        ))),
    }
}

fn check_subjects(subjects: Option<&Value>) -> OxiResult<()> {
    let Some(subjects) = subjects.and_then(Value::as_array) else {
        return Ok(());
    };
    for (index, subject) in subjects.iter().enumerate() {
        let text = |key: &str| subject.get(key).and_then(Value::as_str).unwrap_or_default();
        let at = format!("subjects[{index}]");
        if text("name").is_empty() {
            return Err(OxiError::validation(format!("{at} has no name")));
        }
        let group = subject.get("apiGroup").and_then(Value::as_str);
        match text("kind") {
            "User" | "Group" => check_group(subject.get("apiGroup"), &at)?,
            "ServiceAccount" => {
                if text("namespace").is_empty() {
                    return Err(OxiError::validation(format!(
                        "{at} is a ServiceAccount and needs a namespace"
                    )));
                }
                if group.is_some_and(|group| !group.is_empty()) {
                    return Err(OxiError::validation(format!(
                        "{at} is a ServiceAccount, which has no apiGroup"
                    )));
                }
            }
            other => {
                return Err(OxiError::validation(format!(
                    "{at} has kind {other:?}; it must be User, Group or ServiceAccount"
                )));
            }
        }
    }
    Ok(())
}
