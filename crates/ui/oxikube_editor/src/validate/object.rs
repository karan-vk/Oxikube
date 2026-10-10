//! Mappings: unknown fields, `required`, per-property recursion and `embedded-resource`.

use std::ops::Range;

use oxikube_domain::schema::{AdditionalProperties, JsonSchema};

use super::diagnostic::{DiagnosticCode, Severity};
use super::scalar::{ValueType, scalar_type};
use super::walk::{Walk, describe, find_entry, quote};
use crate::yaml::{NodeId, NodeKind, Role};

impl Walk<'_> {
    pub(super) fn mapping(&mut self, id: NodeId, schema: &JsonSchema, is_root: bool) {
        if !self.check_type(id, schema, ValueType::Object, "") {
            return;
        }
        let (doc, text) = (self.doc, self.text);
        // Kubernetes schemas are structural: an object that lists properties rejects (or prunes)
        // the rest, unless it preserves unknown fields. Free-form objects list none.
        let closed = !schema.xk8s.preserve_unknown_fields
            && match schema.additional_properties {
                AdditionalProperties::Forbidden => true,
                AdditionalProperties::Allowed => !schema.properties.is_empty(),
                AdditionalProperties::Schema(_) => false,
            };
        for (key, value) in doc.entries(id) {
            if !matches!(doc.node(key).kind, NodeKind::Scalar(_)) {
                continue;
            }
            let name = doc.scalar_value(key, text);
            if (is_root && self.opts.skip_status && name == "status") || name == "<<" {
                continue;
            }
            if let Some(property) = schema.properties.get(name) {
                if let Some(value) = value {
                    self.walk(value, property, false);
                }
            } else if let AdditionalProperties::Schema(each) = &schema.additional_properties {
                if let Some(value) = value {
                    self.walk(value, each, false);
                }
            } else if closed {
                self.unknown_field(key, name, schema);
            }
        }
        for name in &schema.required {
            if is_root && self.opts.skip_status && name == "status" {
                continue;
            }
            if find_entry(doc, text, id, name).is_none() {
                self.missing(id, name);
            }
        }
        if schema.xk8s.embedded_resource {
            self.embedded_resource(id);
        }
    }

    /// Reports `required` for the key `name` missing from mapping `id`, unless a syntax error
    /// may have eaten it.
    fn missing(&mut self, id: NodeId, name: &str) {
        if self.near_error(&self.doc.node(id).span) {
            return;
        }
        let span = self.owner_span(id);
        self.report(
            id,
            span,
            Severity::Error,
            DiagnosticCode::Required,
            format!("missing required field {}", quote(name)),
        );
    }

    /// Where to underline a problem with the object as a whole: the key it is the value of,
    /// else its first key, else the object itself (`{}`).
    fn owner_span(&self, id: NodeId) -> Range<usize> {
        let node = self.doc.node(id);
        match node.role {
            Role::Value { key } => self.doc.node(key).span.clone(),
            _ => self.doc.entries(id).next().map_or_else(
                || node.span.clone(),
                |(key, _)| self.doc.node(key).span.clone(),
            ),
        }
    }

    /// `x-kubernetes-embedded-resource`: the object is a whole Kubernetes object, so it names
    /// its `apiVersion` and `kind` as strings. (The embedded kind's own schema is the caller's
    /// to look up; `validate` has only this one.)
    fn embedded_resource(&mut self, id: NodeId) {
        for field in ["apiVersion", "kind"] {
            let Some((_, value)) = find_entry(self.doc, self.text, id, field) else {
                self.missing(id, field);
                continue;
            };
            let Some(value) = value else { continue };
            let text = self.doc.scalar_value(value, self.text);
            let found = match self.doc.node(value).kind {
                NodeKind::Mapping(_) => ValueType::Object,
                NodeKind::Sequence(_) => ValueType::Array,
                NodeKind::Alias { .. } => continue,
                NodeKind::Scalar(style) => scalar_type(style, text),
            };
            match found {
                ValueType::String => {}
                ValueType::Null => self.missing(id, field),
                _ => self.report_node(
                    value,
                    Severity::Error,
                    DiagnosticCode::TypeMismatch,
                    format!("expected string, found {}", describe(found, text)),
                ),
            }
        }
    }
}
