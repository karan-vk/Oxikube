//! [`Unit`]s: the objects of a buffer the validator checks, each against its own kind's schema.
//!
//! A document is one object, except a `kind: List` (`kubectl get -o yaml`) and a root sequence (a
//! JSON array of manifests): there every item is an object of its own kind, and the list itself has
//! no schema worth checking. Splitting a buffer into units is what lets a large one be validated
//! in chunks, the visible objects first (E10-S11).

use std::ops::Range;

use oxikube_domain::ids::Gvk;

use super::scalar::{ValueType, scalar_type};
use super::walk::find_entry;
use crate::yaml::{DocTree, NodeId, NodeKind, ParseResult};

/// One object to validate: a document's root mapping, or an item of a list document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    /// Index of its document in [`ParseResult::docs`].
    pub doc: usize,
    /// The object's mapping node in that document.
    pub node: NodeId,
    /// The object's bytes in the buffer.
    pub span: Range<usize>,
    /// The kind it declares (`apiVersion` and `kind`), whose schema it is checked against.
    pub gvk: Gvk,
}

/// Every object of `parsed` that declares its kind, in buffer order: each document's root, and
/// for a list document (or a root sequence) each of its mapping items instead. Objects without a non-empty string
/// `apiVersion` and `kind` are skipped (nothing to check them against).
#[must_use]
pub fn units(parsed: &ParseResult) -> Vec<Unit> {
    let mut out = Vec::new();
    for (index, doc) in parsed.docs().iter().enumerate() {
        let Some(root) = doc.root() else {
            continue;
        };
        let unit = |node: NodeId| {
            node_gvk(parsed, doc, node).map(|gvk| Unit {
                doc: index,
                node,
                span: doc.node(node).span.clone(),
                gvk,
            })
        };
        match item_container(parsed, doc, root) {
            Some(items) => out.extend(doc.children(items).filter_map(unit)),
            None => out.extend(unit(root)),
        }
    }
    out
}

/// The kind a mapping declares: its `apiVersion` and `kind`, when both are non-empty strings.
#[must_use]
pub fn node_gvk(parsed: &ParseResult, doc: &DocTree, node: NodeId) -> Option<Gvk> {
    if !matches!(doc.node(node).kind, NodeKind::Mapping(_)) {
        return None;
    }
    Some(Gvk::from_api_version(
        string_field(parsed, doc, node, "apiVersion")?,
        string_field(parsed, doc, node, "kind")?,
    ))
}

/// The sequence whose items are the objects: the root itself when it is a sequence, or the
/// `items` of a root `v1` `List`.
fn item_container(parsed: &ParseResult, doc: &DocTree, root: NodeId) -> Option<NodeId> {
    if matches!(doc.node(root).kind, NodeKind::Sequence(_)) {
        return Some(root);
    }
    let is_list = node_gvk(parsed, doc, root)
        .is_some_and(|gvk| gvk.group.is_empty() && &*gvk.version == "v1" && &*gvk.kind == "List");
    if !is_list {
        return None;
    }
    let items = find_entry(doc, parsed.text(), root, "items")?.1?;
    matches!(doc.node(items).kind, NodeKind::Sequence(_)).then_some(items)
}

/// The value of `name` in the mapping `map` when it is a non-empty string.
fn string_field<'p>(
    parsed: &'p ParseResult,
    doc: &'p DocTree,
    map: NodeId,
    name: &str,
) -> Option<&'p str> {
    if !matches!(doc.node(map).kind, NodeKind::Mapping(_)) {
        return None;
    }
    let value = find_entry(doc, parsed.text(), map, name)?.1?;
    let NodeKind::Scalar(style) = doc.node(value).kind else {
        return None;
    };
    let text = doc.scalar_value(value, parsed.text());
    (!text.is_empty() && scalar_type(style, text) == ValueType::String).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml::parse;

    fn kinds(text: &str) -> Vec<String> {
        units(&parse(text))
            .iter()
            .map(|u| u.gvk.kind.to_string())
            .collect()
    }

    #[test]
    fn documents_are_units() {
        let text =
            "apiVersion: v1\nkind: Pod\n---\napiVersion: apps/v1\nkind: Deployment\n---\nfoo: 1\n";
        assert_eq!(kinds(text), ["Pod", "Deployment"]);
        let parsed = parse(text);
        let all = units(&parsed);
        assert_eq!(all[1].doc, 1);
        assert_eq!(all[1].gvk, Gvk::new("apps", "v1", "Deployment"));
        assert_eq!(&text[all[0].span.clone()], "apiVersion: v1\nkind: Pod");
    }

    #[test]
    fn list_items_are_units_and_the_list_is_not() {
        let text = "apiVersion: v1\nkind: List\nitems:\n- apiVersion: v1\n  kind: Pod\n  metadata: {name: a}\n- apiVersion: v1\n  kind: Service\n- not: an object\n";
        assert_eq!(kinds(text), ["Pod", "Service"]);
        let parsed = parse(text);
        let pod = &units(&parsed)[0];
        assert_eq!(pod.doc, 0);
        assert!(text[pod.span.clone()].starts_with("apiVersion: v1\n  kind: Pod"));
    }

    #[test]
    fn a_root_sequence_is_its_items_and_a_list_without_items_is_one_object() {
        let array = "- apiVersion: v1\n  kind: Pod\n- apiVersion: v1\n  kind: Service\n- 3\n";
        assert_eq!(kinds(array), ["Pod", "Service"]);
        let empty = "apiVersion: v1\nkind: List\nspec: {}\n";
        assert_eq!(kinds(empty), ["List"]);
    }
}
