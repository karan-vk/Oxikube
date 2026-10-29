//! [`ApplyPlan`]: documents in apply order, with the edges a dry run needs.

use serde_json::Value;

use super::rank::kind_rank;
use crate::ids::{Gvk, Scope};
use crate::manifests::ManifestDoc;

/// One document in a plan.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanItem {
    /// The document, with `metadata.namespace` defaulted when its kind is namespaced.
    pub doc: ManifestDoc,
    /// Its rank in the kind table (lower applies first).
    pub kind_rank: u8,
    /// Indices into [`ApplyPlan::items`] of the items this one needs to exist first: the
    /// Namespace it lives in and the CustomResourceDefinition that defines its kind, when the plan
    /// creates them. Always lower than this item's own index. A dry run of an item with such an
    /// edge fails on a clean cluster until the earlier item is applied.
    pub depends_on: Vec<usize>,
}

/// Documents in the order to apply them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ApplyPlan {
    /// Items by ascending kind rank; input order within one rank.
    pub items: Vec<PlanItem>,
}

impl ApplyPlan {
    /// Orders `docs` by kind rank (stable: input order within a rank), defaults
    /// `metadata.namespace` to `target_namespace` for namespaced kinds that name none, and records
    /// `depends_on` edges.
    ///
    /// `scope` answers from the discovery registry, which the domain does not own. A kind it does
    /// not know falls back to the `spec.scope` of a CustomResourceDefinition in the plan, else it
    /// is left alone: no defaulting, since guessing would put a cluster-scoped object in a
    /// namespace.
    pub fn build(
        docs: Vec<ManifestDoc>,
        target_namespace: Option<&str>,
        scope: impl Fn(&Gvk) -> Option<Scope>,
    ) -> Self {
        let crd_scopes: Vec<(String, String, Scope)> = docs.iter().filter_map(crd_scope).collect();
        let mut items: Vec<PlanItem> = docs
            .into_iter()
            .map(|mut doc| {
                let gvk = gvk_of(&doc);
                if let Some(namespace) = target_namespace {
                    let scope = scope(&gvk).or_else(|| {
                        crd_scopes
                            .iter()
                            .find(|(group, kind, _)| *group == *gvk.group && *kind == *gvk.kind)
                            .map(|(_, _, scope)| *scope)
                    });
                    if scope == Some(Scope::Namespaced) {
                        default_namespace(&mut doc.json, namespace);
                    }
                }
                PlanItem {
                    kind_rank: kind_rank(&gvk),
                    doc,
                    depends_on: Vec::new(),
                }
            })
            .collect();
        // `sort_by_key` is stable.
        items.sort_by_key(|item| item.kind_rank);
        let objects: Vec<&Value> = items.iter().map(|item| &item.doc.json).collect();
        let edges = depends_on(&objects);
        for (item, depends_on) in items.iter_mut().zip(edges) {
            item.depends_on = depends_on;
        }
        Self { items }
    }
}

/// For each of `objects` (in the order given), the indices of the earlier ones it needs to exist
/// first: the Namespace it names and the CustomResourceDefinition that defines its kind. The one
/// implementation of [`PlanItem::depends_on`], also used where documents keep their own order (a
/// multi-document editor buffer). An object needs nothing from later ones.
pub fn depends_on(objects: &[&Value]) -> Vec<Vec<usize>> {
    let gvks: Vec<Gvk> = objects.iter().map(|object| gvk_of_json(object)).collect();
    (0..objects.len())
        .map(|index| dependencies(objects, &gvks, index))
        .collect()
}

fn gvk_of(doc: &ManifestDoc) -> Gvk {
    gvk_of_json(&doc.json)
}

fn gvk_of_json(object: &Value) -> Gvk {
    let text = |key: &str| object.get(key).and_then(Value::as_str).unwrap_or_default();
    Gvk::from_api_version(text("apiVersion"), text("kind"))
}

fn name_of(object: &Value) -> Option<&str> {
    object.pointer("/metadata/name").and_then(Value::as_str)
}

fn namespace_of(object: &Value) -> Option<&str> {
    object
        .pointer("/metadata/namespace")
        .and_then(Value::as_str)
        .filter(|namespace| !namespace.is_empty())
}

fn default_namespace(json: &mut Value, namespace: &str) {
    let Some(metadata) = json.get_mut("metadata").and_then(Value::as_object_mut) else {
        return;
    };
    let unset = metadata
        .get("namespace")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty);
    if unset {
        metadata.insert("namespace".into(), Value::String(namespace.into()));
    }
}

fn is_crd(gvk: &Gvk) -> bool {
    &*gvk.group == "apiextensions.k8s.io" && &*gvk.kind == "CustomResourceDefinition"
}

fn is_namespace(gvk: &Gvk) -> bool {
    gvk.group.is_empty() && &*gvk.kind == "Namespace"
}

/// `(group, kind, scope)` a CRD document defines.
fn crd_scope(doc: &ManifestDoc) -> Option<(String, String, Scope)> {
    if !is_crd(&gvk_of(doc)) {
        return None;
    }
    let spec_text = |pointer: &str| {
        doc.json
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(String::from)
    };
    let scope = match spec_text("/spec/scope")?.as_str() {
        "Cluster" => Scope::Cluster,
        "Namespaced" => Scope::Namespaced,
        _ => return None,
    };
    Some((
        spec_text("/spec/group")?,
        spec_text("/spec/names/kind")?,
        scope,
    ))
}

/// The earlier objects that `objects[index]` needs.
fn dependencies(objects: &[&Value], gvks: &[Gvk], index: usize) -> Vec<usize> {
    let gvk = &gvks[index];
    let namespace = namespace_of(objects[index]);
    (0..index)
        .filter(|&earlier| {
            let other = objects[earlier];
            let other_gvk = &gvks[earlier];
            if is_namespace(other_gvk) {
                // A Namespace needs no namespace; a cluster-scoped object that names one ignores it
                // on the server, but waiting for it is harmless.
                return !is_namespace(gvk) && namespace.is_some() && namespace == name_of(other);
            }
            is_crd(other_gvk) && defines(other, gvk)
        })
        .collect()
}

fn defines(crd: &Value, gvk: &Gvk) -> bool {
    let at = |pointer: &str| crd.pointer(pointer).and_then(Value::as_str);
    at("/spec/group") == Some(&*gvk.group) && at("/spec/names/kind") == Some(&*gvk.kind)
}
