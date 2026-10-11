//! Where things are in the rendered text: the `status` ranges to dim, the placeholders standing
//! for Secret values and the `metadata` key a hidden part was removed from. Found with the
//! spanned YAML model (E10-S02) over the text just written, so the offsets are those of the
//! buffer the editor gets.

use std::ops::Range;

use super::render::{HiddenRegion, Left, MASKED, MaskedRegion, RenderedManifest, is_list};
use crate::yaml::{DocTree, NodeId, NodeKind, parse};

/// The regions of `text`, written for the objects `left` describes.
pub(super) fn locate(text: String, left: &[Left]) -> RenderedManifest {
    let parsed = parse(&text);
    let mut out = RenderedManifest::default();
    if let Some(doc) = parsed.docs().first() {
        let objects = objects(doc, &text);
        for (index, object) in objects.iter().enumerate() {
            if let Some(range) = entry_range(doc, *object, "status", &text) {
                out.dimmed.push(range);
            }
            out.masked.extend(masked(doc, *object, &text));
            for hidden in left.iter().filter(|hidden| hidden.object == index) {
                let at = entry_key(doc, *object, "metadata", &text)
                    .map_or(0, |key| doc.node(key).span.start);
                out.hidden.push(HiddenRegion {
                    kind: hidden.kind,
                    at,
                    count: hidden.count,
                });
            }
        }
    }
    out.text = text;
    out
}

/// The mapping of each object in the document: the root, or the items of a List.
fn objects(doc: &DocTree, text: &str) -> Vec<NodeId> {
    let Some(root) = doc.root() else {
        return Vec::new();
    };
    if !matches!(doc.node(root).kind, NodeKind::Mapping(_)) {
        return Vec::new();
    }
    let list = entry_value(doc, root, "kind", text)
        .is_some_and(|kind| is_list(doc.scalar_value(kind, text)));
    if list
        && let Some(items) = entry_value(doc, root, "items", text)
        && matches!(doc.node(items).kind, NodeKind::Sequence(_))
    {
        return doc
            .children(items)
            .filter(|item| matches!(doc.node(*item).kind, NodeKind::Mapping(_)))
            .collect();
    }
    vec![root]
}

fn entry_key(doc: &DocTree, map: NodeId, name: &str, text: &str) -> Option<NodeId> {
    doc.entries(map)
        .find(|(key, _)| doc.scalar_value(*key, text) == name)
        .map(|(key, _)| key)
}

fn entry_value(doc: &DocTree, map: NodeId, name: &str, text: &str) -> Option<NodeId> {
    doc.entries(map)
        .find(|(key, _)| doc.scalar_value(*key, text) == name)
        .and_then(|(_, value)| value)
}

/// The bytes of the entry `name` of `map`, key and value.
fn entry_range(doc: &DocTree, map: NodeId, name: &str, text: &str) -> Option<Range<usize>> {
    let (key, value) = doc
        .entries(map)
        .find(|(key, _)| doc.scalar_value(*key, text) == name)?;
    let start = doc.node(key).span.start;
    let end = value.map_or(doc.node(key).span.end, |value| doc.node(value).span.end);
    Some(start..end.max(start))
}

/// The placeholders of a Secret's `data` and `stringData`.
fn masked(doc: &DocTree, object: NodeId, text: &str) -> Vec<MaskedRegion> {
    let is_secret = entry_value(doc, object, "kind", text)
        .is_some_and(|kind| doc.scalar_value(kind, text) == "Secret");
    if !is_secret {
        return Vec::new();
    }
    let mut regions = Vec::new();
    for field in ["data", "stringData"] {
        let Some(values) = entry_value(doc, object, field, text) else {
            continue;
        };
        if !matches!(doc.node(values).kind, NodeKind::Mapping(_)) {
            continue;
        }
        for (key, value) in doc.entries(values) {
            let Some(value) = value else { continue };
            if matches!(doc.node(value).kind, NodeKind::Scalar(_))
                && doc.scalar_value(value, text) == MASKED
            {
                regions.push(MaskedRegion {
                    range: doc.node(value).span.clone(),
                    key: doc.scalar_value(key, text).to_owned(),
                });
            }
        }
    }
    regions
}

/// The placeholders still in `doc` (any object of it that is a Secret).
pub(super) fn placeholders(doc: &DocTree, text: &str) -> Vec<MaskedRegion> {
    objects(doc, text)
        .into_iter()
        .flat_map(|object| masked(doc, object, text))
        .collect()
}
