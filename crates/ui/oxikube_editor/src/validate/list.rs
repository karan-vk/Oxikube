//! Sequences: item schemas and the `x-kubernetes-list-type` duplicate checks.

use std::collections::HashSet;

use oxikube_domain::schema::JsonSchema;

use super::diagnostic::{DiagnosticCode, Severity};
use super::scalar::{ValueType, scalar_type};
use super::walk::Walk;
use crate::yaml::{NodeId, NodeKind};

impl Walk<'_> {
    pub(super) fn sequence(&mut self, id: NodeId, schema: &JsonSchema) {
        if !self.check_type(id, schema, ValueType::Array, "") {
            return;
        }
        if let Some(items) = &schema.items {
            let doc = self.doc;
            for item in doc.children(id) {
                self.walk(item, items, false);
            }
        }
        match schema.xk8s.list_type.as_deref() {
            Some("map") if !schema.xk8s.list_map_keys.is_empty() => {
                self.duplicate_keys(id, &schema.xk8s.list_map_keys);
            }
            Some("set") => self.duplicate_items(id),
            _ => {}
        }
    }

    /// `list-type: map`: no two items may have the same values for all `keys`. An item missing a
    /// key is skipped (the server may default it).
    fn duplicate_keys(&mut self, id: NodeId, keys: &[String]) {
        let (doc, text) = (self.doc, self.text);
        let mut seen: HashSet<Vec<&str>> = HashSet::new();
        for item in doc.children(id) {
            if !matches!(doc.node(item).kind, NodeKind::Mapping(_)) {
                continue;
            }
            let values: Option<Vec<(NodeId, &str)>> = keys
                .iter()
                .map(|wanted| {
                    let (_, value) = doc
                        .entries(item)
                        .find(|(key, _)| doc.scalar_value(*key, text) == wanted)?;
                    let value = value?;
                    let node = doc.node(value);
                    (matches!(node.kind, NodeKind::Scalar(_)) && !node.is_implicit_null())
                        .then(|| (value, doc.scalar_value(value, text)))
                })
                .collect();
            let Some(values) = values else { continue };
            let shown: Vec<&str> = values.iter().map(|(_, v)| *v).collect();
            if seen.insert(shown.clone()) {
                continue;
            }
            let message = format!(
                "duplicate list entry with the same {} ({})",
                keys.join(", "),
                shown.join(", ")
            );
            let (first, _) = values[0];
            let span = doc.node(first).span.clone();
            self.report(
                first,
                span,
                Severity::Warning,
                DiagnosticCode::DuplicateKey,
                message,
            );
        }
    }

    /// `list-type: set`: no two scalar items may be equal.
    fn duplicate_items(&mut self, id: NodeId) {
        let (doc, text) = (self.doc, self.text);
        let mut seen: HashSet<(bool, &str)> = HashSet::new();
        for item in doc.children(id) {
            let NodeKind::Scalar(style) = doc.node(item).kind else {
                continue;
            };
            let value = doc.scalar_value(item, text);
            let ty = scalar_type(style, value);
            if ty == ValueType::Null || seen.insert((ty == ValueType::String, value)) {
                continue;
            }
            let span = doc.node(item).span.clone();
            self.report(
                item,
                span,
                Severity::Warning,
                DiagnosticCode::DuplicateItem,
                format!("duplicate list item {value:?}"),
            );
        }
    }
}
