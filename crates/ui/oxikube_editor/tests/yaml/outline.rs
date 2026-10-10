//! Test helpers: a readable outline of a parse, and invariants every parse must satisfy.

#![allow(dead_code, reason = "each test binary uses a subset")]

use oxikube_editor::yaml::{NodeId, NodeKind, ParseResult, Role};

/// One line per node: `doc | path | kind | source text`.
pub fn outline(result: &ParseResult) -> String {
    let text = result.text();
    let mut out = String::new();
    for doc in result.docs() {
        for (i, node) in doc.nodes().iter().enumerate() {
            let id = NodeId::new(i);
            let kind = match node.kind {
                NodeKind::Mapping(s) => format!("map {s:?}"),
                NodeKind::Sequence(s) => format!("seq {s:?}"),
                NodeKind::Scalar(s) => format!("{s:?} {:?}", doc.scalar_value(id, text)),
                NodeKind::Alias { target } => format!("alias -> {:?}", target.map(NodeId::index)),
            };
            let role = if node.role == Role::Key { " (key)" } else { "" };
            out.push_str(&format!(
                "{} | {}{role} | {kind} | {:?}\n",
                doc.index,
                doc.path_of(id, text),
                &text[node.span.clone()],
            ));
        }
    }
    out
}

/// Structural invariants: spans inside the text and on char boundaries, children inside their
/// parent, pre-order sorted by start, documents in order, every node found again by its path
/// (unless a reported duplicate key shadows it) and by its own start offset.
pub fn check_invariants(result: &ParseResult) {
    let text = result.text();
    let mut last_doc_start = 0;
    for doc in result.docs() {
        assert!(doc.span.start >= last_doc_start, "documents out of order");
        assert!(doc.span.start <= doc.span.end && doc.span.end <= text.len());
        last_doc_start = doc.span.start;
        let mut last_start = 0;
        for (i, node) in doc.nodes().iter().enumerate() {
            let id = NodeId::new(i);
            let span = &node.span;
            assert!(span.start <= span.end && span.end <= text.len(), "{span:?}");
            assert!(text.is_char_boundary(span.start) && text.is_char_boundary(span.end));
            assert!(span.start >= last_start, "pre-order not sorted at {span:?}");
            last_start = span.start;
            if let Some(parent) = node.parent {
                let outer = &doc.node(parent).span;
                assert!(
                    outer.start <= span.start && span.end <= outer.end,
                    "{span:?} outside {outer:?}"
                );
                assert!(doc.children(parent).any(|c| c == id));
            }
            if !span.is_empty() {
                let found = doc.node_at(span.start).expect("node at its own start");
                let mut up = Some(found);
                while up.is_some_and(|u| u != id) {
                    up = doc.node(up.unwrap()).parent;
                }
                assert_eq!(up, Some(id), "node_at({}) not inside {span:?}", span.start);
            }
            let path = doc.path_of(id, text);
            let duplicates = result
                .diagnostics()
                .iter()
                .any(|d| d.message.starts_with("duplicate"));
            assert!(
                doc.lookup(&path, text).is_some() || duplicates,
                "path {path} does not resolve"
            );
        }
    }
    for diag in result.diagnostics() {
        assert!(diag.span.start <= diag.span.end && diag.span.end <= text.len());
        assert!(text.is_char_boundary(diag.span.start) && text.is_char_boundary(diag.span.end));
    }
    for offset in 0..=text.len() {
        if text.is_char_boundary(offset) {
            let _ = result.offset_to_path(offset);
            let _ = result.key_at(offset);
        }
    }
}
