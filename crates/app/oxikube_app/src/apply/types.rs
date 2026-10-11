//! What [`ApplyService`](super::ApplyService) takes and gives: [`ApplyRequest`],
//! [`ApplyPreview`], [`ApplyReport`] and [`ApplyError`].

use std::fmt;

use oxikube_domain::FieldCause;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use serde_json::Value;

use crate::actions::object_label;

/// What to apply where. The manifest may hold Secret data: it lives in memory only and is
/// never logged (the `Debug` form leaves it out).
#[derive(Clone, PartialEq, Eq)]
pub struct ApplyRequest {
    /// The cluster to apply to.
    pub cluster: ClusterId,
    /// The manifest text, YAML or JSON, of one object.
    pub manifest: String,
    /// The namespace a namespaced object without one goes to; `None` is `default`.
    pub namespace: Option<String>,
    /// The server-side apply field manager (the `editor.field_manager` setting); `None` is
    /// `oxikube`.
    pub field_manager: Option<String>,
    /// Take over fields other managers own. Off unless the user chose it after seeing the
    /// conflicts.
    pub force: bool,
}

impl ApplyRequest {
    /// Applies `manifest` to `cluster` as `oxikube`, without force.
    pub fn new(cluster: ClusterId, manifest: impl Into<String>) -> Self {
        Self {
            cluster,
            manifest: manifest.into(),
            namespace: None,
            field_manager: None,
            force: false,
        }
    }

    /// The same request with a default namespace.
    #[must_use]
    pub fn in_namespace(mut self, namespace: Option<String>) -> Self {
        self.namespace = namespace;
        self
    }

    /// The same request under another field manager.
    #[must_use]
    pub fn managed_by(mut self, manager: Option<String>) -> Self {
        self.field_manager = manager;
        self
    }

    /// The same request taking over conflicting fields (after the user chose "Force").
    #[must_use]
    pub fn forced(mut self) -> Self {
        self.force = true;
        self
    }

    /// The `resource::Apply` command for this request.
    pub(crate) fn command(&self, resource_version: Option<String>) -> Command {
        Command::ResourceApply {
            cluster: self.cluster.clone(),
            namespace: self.namespace.clone(),
            manifest: self.manifest.clone(),
            field_manager: self.field_manager.clone(),
            force: self.force,
            resource_version,
        }
    }
}

impl fmt::Debug for ApplyRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApplyRequest")
            .field("cluster", &self.cluster)
            .field("manifest_bytes", &self.manifest.len())
            .field("namespace", &self.namespace)
            .field("field_manager", &self.field_manager)
            .field("force", &self.force)
            .finish()
    }
}

/// How the object reaches the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyMethod {
    /// Server-side apply under the field manager.
    ServerSide,
    /// The fallback where the server cannot server-side apply the kind: a replace (PUT) carrying
    /// the reviewed `resourceVersion`.
    Replace,
    /// The fallback for an object that does not exist yet: a create (POST).
    Create,
}

impl ApplyMethod {
    /// The name in the handler's output (`server_side`, `replace`, `create`).
    pub fn as_str(self) -> &'static str {
        match self {
            ApplyMethod::ServerSide => "server_side",
            ApplyMethod::Replace => "replace",
            ApplyMethod::Create => "create",
        }
    }

    /// The method named `name` in the handler's output.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        [Self::ServerSide, Self::Replace, Self::Create]
            .into_iter()
            .find(|method| method.as_str() == name)
    }

    /// Whether this is a fallback for a server that cannot server-side apply.
    pub fn is_fallback(self) -> bool {
        self != ApplyMethod::ServerSide
    }
}

/// The server's dry run of an apply: what to show before anything is written.
///
/// Secret values are already masked in [`live`](Self::live) and [`result`](Self::result).
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyPreview {
    /// The request it previews; [`ApplyService::apply`](super::ApplyService::apply) sends it.
    pub request: ApplyRequest,
    /// The object it applies.
    pub target: ResourceRef,
    /// How it will be sent.
    pub method: ApplyMethod,
    /// The live object, `None` when the apply creates it.
    pub live: Option<Value>,
    /// The object as the server would store it.
    pub result: Value,
    /// The live object's `metadata.resourceVersion`; the replace fallback carries it.
    pub resource_version: Option<String>,
}

impl ApplyPreview {
    /// "Deployment default/web".
    pub fn label(&self) -> String {
        object_label(&self.target)
    }

    /// Whether the apply creates the object.
    pub fn creates(&self) -> bool {
        self.live.is_none()
    }
}

/// What an apply did.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyReport {
    /// The object.
    pub target: ResourceRef,
    /// How it was sent: [`ApplyMethod::is_fallback`] says the server could not server-side
    /// apply it.
    pub method: ApplyMethod,
    /// The object as the server stored it (Secret values masked).
    pub object: Value,
    /// One line for a toast: "Applied Deployment default/web".
    pub message: String,
}

/// A field another manager owns, from a server-side apply conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldConflict {
    /// The field, in the server's notation (`.spec.replicas`).
    pub field: String,
    /// The manager that owns it.
    pub manager: Option<String>,
    /// When that manager last wrote the object (`managedFields[].time`), when known.
    pub applied_at: Option<String>,
}

/// Why an apply did not happen. Messages carry names and fields, never values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApplyError {
    /// The cluster is read-only: nothing was sent.
    #[error("{context} is read-only: the manifest was not applied")]
    ReadOnly {
        /// The cluster's context name.
        context: String,
    },
    /// Fields of the manifest are owned by other field managers. Apply again with
    /// [`ApplyRequest::forced`] to take them over, or drop them from the manifest.
    #[error("{} field(s) are managed by {}", .0.len(), managers(.0))]
    Conflicts(Vec<FieldConflict>),
    /// The object changed on the server since it was reviewed (the replace fallback's
    /// `resourceVersion` is stale): preview again.
    #[error("the object changed on the server since the preview: {0}")]
    Stale(String),
    /// The server (or the manifest check before it) rejected the manifest. `causes` name the
    /// fields, for the editor to mark.
    #[error("{message}")]
    Invalid {
        /// The server's message, redacted.
        message: String,
        /// One cause per rejected field the server named.
        causes: Vec<FieldCause>,
    },
    /// The guard asked for more than the preview confirms (the policy changed): nothing was
    /// sent.
    #[error("this apply needs a stronger confirmation than the preview; try again")]
    Unconfirmed,
    /// Anything else, with the error's message.
    #[error("{0}")]
    Failed(String),
}

/// "kubectl and helm".
fn managers(conflicts: &[FieldConflict]) -> String {
    let mut names: Vec<&str> = Vec::new();
    for name in conflicts.iter().filter_map(|c| c.manager.as_deref()) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    match names.as_slice() {
        [] => "another manager".to_owned(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
