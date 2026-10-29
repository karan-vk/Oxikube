//! What [`CopyService`](super::CopyService) takes and gives: [`CopyRequest`], [`CopyPreview`],
//! [`CopyChoice`], [`CopyOutcome`] and [`CopyError`].

use oxikube_domain::FieldCause;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::safety::ConfirmTier;
use serde_json::Value;

use crate::actions::object_label;
use crate::apply::{Change, FieldConflict};

/// What to copy where: an object of a source cluster, and the object the copy becomes.
///
/// `target` names the target cluster, the namespace (none for a cluster-scoped kind) and the name
/// the copy takes; its kind is the source's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyRequest {
    /// The object to copy.
    pub source: ResourceRef,
    /// The copy.
    pub target: ResourceRef,
}

impl CopyRequest {
    /// A copy of `source` into `namespace` of `cluster` under the same name. `namespace` is
    /// ignored for a cluster-scoped `source`.
    pub fn to(source: ResourceRef, cluster: ClusterId, namespace: Option<&str>) -> Self {
        let target = match (source.namespace(), namespace) {
            (Some(_), Some(namespace)) => {
                ResourceRef::namespaced(cluster, source.gvk.clone(), namespace, &*source.name)
            }
            _ => ResourceRef::cluster_scoped(cluster, source.gvk.clone(), &*source.name),
        };
        Self { source, target }
    }

    /// The same copy under another name.
    #[must_use]
    pub fn renamed(&self, name: &str) -> Self {
        let target = ResourceRef::new(
            self.target.cluster.clone(),
            self.target.gvk.clone(),
            self.target.namespace().map(std::sync::Arc::from),
            name,
        );
        Self {
            source: self.source.clone(),
            target,
        }
    }
}

/// A reference of the copied object that the target does not have: a warning, never an error
/// (copying dependencies is a separate feature).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingReference {
    /// The kind: "ConfigMap", "Secret", "PersistentVolumeClaim", "ServiceAccount",
    /// "PriorityClass".
    pub kind: String,
    /// The name the object points at.
    pub name: String,
    /// The namespace it is looked up in; `None` for a cluster-scoped kind.
    pub namespace: Option<String>,
}

impl MissingReference {
    /// "ConfigMap web-config is not in namespace shop on the target".
    pub fn message(&self) -> String {
        match &self.namespace {
            Some(namespace) => format!(
                "{} {} is not in namespace {namespace} on the target",
                self.kind, self.name
            ),
            None => format!("{} {} is not on the target", self.kind, self.name),
        }
    }
}

/// The target cluster's dry run of a copy: what to show before anything is written.
///
/// Secret values are already masked in [`live`](Self::live) and [`result`](Self::result).
#[derive(Debug, Clone, PartialEq)]
pub struct CopyPreview {
    /// The copy it previews.
    pub request: CopyRequest,
    /// The object the copy writes, as the target would name it.
    pub target: ResourceRef,
    /// The object already at the target, `None` when the copy creates it.
    pub live: Option<Value>,
    /// The object as the target would store it.
    pub result: Value,
    /// Fields other managers own at the target, when the object exists: overwriting needs force,
    /// which the caller confirms separately. Empty when nothing clashes.
    pub conflicts: Vec<FieldConflict>,
    /// Paths of the fields the sanitiser dropped from the source (never values).
    pub removed: Vec<String>,
    /// References the target lacks.
    pub missing: Vec<MissingReference>,
    /// The confirmation the target's guard will ask for.
    pub tier: ConfirmTier,
}

impl CopyPreview {
    /// "Deployment shop/web".
    pub fn label(&self) -> String {
        object_label(&self.target)
    }

    /// Whether the target has the object already: the user must choose [`CopyChoice`].
    pub fn exists(&self) -> bool {
        self.live.is_some()
    }

    /// What the copy does to the object at the target.
    pub fn change(&self) -> Change {
        Change::between(self.live.as_ref(), &self.result)
    }
}

/// What to do about an object that exists at the target already. Remembered, if at all, only for
/// the operation that asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyChoice {
    /// Server-side apply over it. `force` takes over fields other managers own: the caller asks
    /// the user for an extra confirmation first.
    Overwrite {
        /// Take over conflicting fields.
        force: bool,
    },
    /// Leave it: nothing is written.
    Skip,
    /// Copy under another name (a DNS-1123 name; `<name>-copy` is the suggestion).
    Rename(String),
}

/// How a copy ended.
#[derive(Debug, Clone, PartialEq)]
pub enum CopyOutcome {
    /// Written at the target.
    Copied {
        /// The object written.
        target: ResourceRef,
        /// Whether it was created (`true`) or an existing object was overwritten.
        created: bool,
        /// "Created Deployment shop/web on kind-b".
        message: String,
    },
    /// The user chose to skip an object that exists: zero writes.
    Skipped {
        /// The object left alone.
        target: ResourceRef,
    },
}

/// Why a copy did not happen. Messages carry names and fields, never values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CopyError {
    /// The target cluster is read-only: nothing was read from the source or written.
    #[error("{context} is read-only: the object was not copied")]
    ReadOnly {
        /// The target cluster's context name.
        context: String,
    },
    /// The cluster could not be connected.
    #[error("could not connect to {0}")]
    NotConnected(String),
    /// The source object could not be read.
    #[error("{0}")]
    SourceUnavailable(String),
    /// The target does not serve the kind at all.
    #[error("{0}")]
    KindNotServed(String),
    /// The target serves the kind, but not the source's apiVersion.
    #[error("{0}")]
    VersionNotServed(String),
    /// The target namespace does not exist. Creating it is a separate, guarded step
    /// ([`CopyService::create_namespace`](super::CopyService::create_namespace)).
    #[error("namespace {0} does not exist on the target")]
    NamespaceMissing(String),
    /// The object exists at the target: choose overwrite, skip or rename.
    #[error("{0} exists on the target already")]
    Exists(String),
    /// Fields of the object at the target are owned by other managers: overwrite again with
    /// force to take them over (after an extra confirmation).
    #[error("{} field(s) are managed by {}", .0.len(), conflict_managers_text(.0))]
    Conflicts(Vec<FieldConflict>),
    /// The name is not a valid DNS-1123 name.
    #[error("{0}")]
    InvalidName(String),
    /// The target (or the check before it) rejected the object; `causes` name the fields. A
    /// field the target treats as immutable (a Deployment's `spec.selector`) is shown as the
    /// server says it.
    #[error("{message}")]
    Invalid {
        /// The server's message, redacted.
        message: String,
        /// One cause per rejected field.
        causes: Vec<FieldCause>,
    },
    /// The guard asked for a stronger confirmation than the caller can give.
    #[error("this copy needs a stronger confirmation than the review; try again")]
    Unconfirmed,
    /// The confirmation was declined: nothing was written.
    #[error("the copy was not confirmed: nothing was written")]
    Declined,
    /// Anything else, with the error's message.
    #[error("{0}")]
    Failed(String),
}

/// "kubectl and helm".
fn conflict_managers_text(conflicts: &[FieldConflict]) -> String {
    match crate::apply::conflict_managers(conflicts).as_slice() {
        [] => "another manager".to_owned(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
