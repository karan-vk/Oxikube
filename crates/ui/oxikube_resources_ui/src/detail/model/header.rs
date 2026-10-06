//! [`Header`]: the identity line of the drawer, identical across kinds.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_app::columns::Tone;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{Age, ObjectMeta};

/// The status chip: the text of the kind's status cell and its tone, the same mapping the table
/// uses (E07-S02), so a pod reads `Running` in green here and in its row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusChip {
    /// What the chip says.
    pub text: String,
    /// How it is coloured.
    pub tone: Tone,
}

/// Kind, name, namespace, creation time and status of the object shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// The kind, for example `Pod`.
    pub kind: Arc<str>,
    /// `metadata.name`.
    pub name: Arc<str>,
    /// `metadata.namespace`; `None` for cluster-scoped objects.
    pub namespace: Option<Arc<str>>,
    /// `metadata.creationTimestamp`, from which the age is read at draw time.
    pub created: Option<Timestamp>,
    /// The status chip, when the kind has a status (or ready) column.
    pub status: Option<StatusChip>,
}

impl Header {
    pub(super) fn new(gvk: &Gvk, meta: &ObjectMeta, status: Option<StatusChip>) -> Self {
        Self {
            kind: gvk.kind.clone(),
            name: meta.name.clone(),
            namespace: meta.namespace.clone(),
            created: meta.creation,
            status,
        }
    }

    /// The age as of `now`, in `kubectl` text (`3d5h`); `None` without a creation time.
    pub fn age(&self, now: Timestamp) -> Option<String> {
        self.created
            .map(|created| Age::between(created, now).to_kubectl_string())
    }
}
