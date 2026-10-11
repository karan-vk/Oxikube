//! [`delete_advice`]: what to tell the user before a namespace is deleted, read from the store.

use std::collections::{BTreeMap, HashMap};

use oxikube_domain::command::{Command, Propagation};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::view::{DeleteNote, NamespaceSummary, delete_notes, is_hnc_descendant};

use crate::store::ResourceStore;

/// A kind whose objects in a namespace are counted for the delete warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentKind {
    /// API group; empty for the core group.
    pub group: &'static str,
    /// API version the store watches.
    pub version: &'static str,
    /// Kind: the key of the counts and the word of the warning.
    pub kind: &'static str,
}

impl ContentKind {
    /// The kind as a [`Gvk`].
    pub fn gvk(&self) -> Gvk {
        Gvk::new(self.group, self.version, self.kind)
    }
}

/// The kinds a delete warning counts when the store has them: the workloads and the objects a
/// user is most afraid to lose with a namespace.
pub const CONTENT_KINDS: &[ContentKind] = &[
    ContentKind {
        group: "",
        version: "v1",
        kind: "Pod",
    },
    ContentKind {
        group: "apps",
        version: "v1",
        kind: "Deployment",
    },
    ContentKind {
        group: "apps",
        version: "v1",
        kind: "StatefulSet",
    },
    ContentKind {
        group: "apps",
        version: "v1",
        kind: "DaemonSet",
    },
    ContentKind {
        group: "batch",
        version: "v1",
        kind: "Job",
    },
    ContentKind {
        group: "batch",
        version: "v1",
        kind: "CronJob",
    },
    ContentKind {
        group: "",
        version: "v1",
        kind: "Service",
    },
    ContentKind {
        group: "",
        version: "v1",
        kind: "PersistentVolumeClaim",
    },
    ContentKind {
        group: "",
        version: "v1",
        kind: "ConfigMap",
    },
    ContentKind {
        group: "",
        version: "v1",
        kind: "Secret",
    },
];

/// The delete dialog's warnings for one namespace, and the counts they rest on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NamespaceAdvice {
    /// The lines to show, most severe first (see [`delete_notes`]).
    pub notes: Vec<DeleteNote>,
    /// What the namespace holds: kind to count, only for kinds the store has watched. These go
    /// into `namespace::Delete` and so into its audit record.
    pub contents: BTreeMap<String, u32>,
}

/// The advice for deleting the namespace `name`, from what `store` already holds: the Namespace
/// object (phase, finalizers, HNC markers), the other namespaces (HNC descendants) and the
/// per-namespace counts of [`CONTENT_KINDS`]. Reads caches only: a kind nobody watches is not
/// counted (and no feed is started for it), and without a store (`None`) the advice is the
/// name-based warning alone.
pub fn delete_advice(store: Option<&ResourceStore>, name: &str) -> NamespaceAdvice {
    let namespace_kind = Gvk::new("", "v1", "Namespace");
    let summary = store
        .and_then(|store| store.cluster_object(&namespace_kind, name))
        .and_then(|object| NamespaceSummary::from_resource(object.resource()?).ok())
        .unwrap_or_else(|| NamespaceSummary::named(name));
    let descendants = store
        .and_then(|store| store.cluster_objects(&namespace_kind))
        .map_or(0, |all| {
            all.iter()
                .filter(|object| {
                    let meta = object.meta();
                    is_hnc_descendant(&meta.labels, &meta.name, name)
                })
                .count()
        });
    let contents: BTreeMap<String, u32> = store
        .map(|store| {
            CONTENT_KINDS
                .iter()
                .filter_map(|kind| {
                    let count = store.count_in_namespace(&kind.gvk(), name)?;
                    Some((
                        kind.kind.to_owned(),
                        u32::try_from(count).unwrap_or(u32::MAX),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    // In the order of CONTENT_KINDS (pods first), not alphabetical.
    let held: Vec<(String, u32)> = CONTENT_KINDS
        .iter()
        .filter_map(|kind| Some((kind.kind.to_owned(), *contents.get(kind.kind)?)))
        .collect();
    NamespaceAdvice {
        notes: delete_notes(&summary, descendants, &held),
        contents,
    }
}

/// The command builder for [`DeleteFlow::with_command`](crate::DeleteFlow::with_command): each
/// namespace is deleted with `namespace::Delete`, carrying the counts [`delete_advice`] found for
/// it (a namespace not in `contents` carries none), so the audit record says what went with it.
pub fn delete_command(
    contents: HashMap<String, BTreeMap<String, u32>>,
) -> impl Fn(&ResourceRef, Propagation) -> Command + Send + Sync + 'static {
    move |target, _propagation| Command::NamespaceDelete {
        target: target.clone(),
        contents: contents.get(&*target.name).cloned().unwrap_or_default(),
    }
}
