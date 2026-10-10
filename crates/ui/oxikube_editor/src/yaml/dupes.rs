//! Duplicate mapping keys: valid to the event parser, an error to YAML 1.2 and to the API server.

use std::collections::HashSet;

use super::node::{NodeId, NodeKind};
use super::result::SyntaxDiagnostic;
use super::tree::DocTree;

/// Reports every key that repeats an earlier key of the same mapping (compared by decoded text,
/// so `a` and `"a"` collide), while `out` holds fewer than `limit` diagnostics. Lookups resolve a
/// duplicated path to the first entry.
pub(crate) fn duplicate_keys(
    doc: &DocTree,
    text: &str,
    out: &mut Vec<SyntaxDiagnostic>,
    limit: usize,
) {
    let mut seen: HashSet<&str> = HashSet::new();
    for (index, node) in doc.nodes().iter().enumerate() {
        if !matches!(node.kind, NodeKind::Mapping(_)) {
            continue;
        }
        seen.clear();
        for (key, _) in doc.entries(NodeId::new(index)) {
            let name = doc.scalar_value(key, text);
            if !seen.insert(name) {
                if out.len() >= limit {
                    return;
                }
                out.push(SyntaxDiagnostic {
                    doc: doc.index,
                    span: doc.node(key).span.clone(),
                    message: format!("duplicate mapping key {name:?}"),
                });
            }
        }
    }
}
