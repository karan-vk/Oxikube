//! [`Item`]: one object of a plan, ready to write, with what names it in results and audit.

use oxikube_domain::apply::{ApplyPlan, PlanItem};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};

use crate::actions::object_label;
use crate::apply::manifest::Manifest;
use crate::apply::write::ObjectWrite;

/// One object of the plan.
pub(super) struct Item {
    /// Its index in the plan's items.
    pub(super) index: usize,
    /// Its kind rank: objects of one rank run together, ranks in order.
    pub(super) rank: u8,
    /// The earlier items it needs (its Namespace, its definition).
    pub(super) depends_on: Vec<usize>,
    /// "ConfigMap shop/web", or "Object 3" for one that names nothing.
    pub(super) label: String,
    /// The write, or why the object cannot be written (it names no kind or no name).
    pub(super) write: Result<ObjectWrite, String>,
}

impl Item {
    /// The items of `plan` for `cluster`, under the field manager `manager`.
    pub(super) fn all(
        plan: ApplyPlan,
        cluster: &ClusterId,
        manager: &str,
        force: bool,
    ) -> Vec<Self> {
        plan.items
            .into_iter()
            .enumerate()
            .map(|(index, item)| Self::new(index, item, cluster, manager, force))
            .collect()
    }

    fn new(index: usize, item: PlanItem, cluster: &ClusterId, manager: &str, force: bool) -> Self {
        let PlanItem {
            doc,
            kind_rank,
            depends_on,
        } = item;
        let write = Manifest::from_object(doc.json)
            .map(|manifest| {
                // The plan put a namespaced object in its namespace; one whose kind nobody knows
                // keeps what it says, and the server decides.
                let namespace = manifest.namespace.clone();
                ObjectWrite {
                    target: manifest.target(cluster.clone(), namespace.as_deref()),
                    body: manifest.body(namespace.as_deref()),
                    manager: manager.to_owned(),
                    force,
                    resource_version: None,
                    reviewed_absent: false,
                }
            })
            .map_err(|error| error.message().to_owned());
        let label = match &write {
            Ok(write) => object_label(&write.target),
            Err(_) => format!("Object {}", index + 1),
        };
        Self {
            index,
            rank: kind_rank,
            depends_on,
            label,
            write,
        }
    }

    /// The object, when it names one.
    pub(super) fn target(&self) -> Option<&ResourceRef> {
        self.write.as_ref().ok().map(|write| &write.target)
    }

    /// Whether the object is a CustomResourceDefinition: its custom resources wait until it is
    /// `Established`.
    pub(super) fn is_crd(&self) -> bool {
        self.target().is_some_and(|target| is_crd(&target.gvk))
    }
}

fn is_crd(gvk: &Gvk) -> bool {
    &*gvk.group == "apiextensions.k8s.io" && &*gvk.kind == "CustomResourceDefinition"
}
