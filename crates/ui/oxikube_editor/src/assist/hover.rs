//! Hover docs: the schema's description of the field under the pointer.

use std::ops::Range;
use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::JsonSchema;

use super::describe::{code_list, enum_text, type_label};
use super::lookup::{Lookup, schema_at};
use crate::validate::document_gvk;
use crate::yaml::{JsonPath, NodeKind, ParseResult, PathSegment};

/// What the schema says about one field, for the hover popup.
#[derive(Clone, Debug, PartialEq)]
pub struct HoverDoc {
    /// The key or value under the pointer.
    pub range: Range<usize>,
    /// The field's path in its document (`spec.containers[0].image`).
    pub path: JsonPath,
    /// The field's name: its key, or `[n]` for a list item.
    pub name: String,
    /// The type (`integer`, `string (enum)`, `array of object`, `map of string`).
    pub type_label: String,
    /// The schema's description.
    pub description: Option<String>,
    /// The allowed values, when the schema lists them.
    pub enum_values: Vec<String>,
    /// The default the API server applies, when the schema states one.
    pub default: Option<String>,
    /// The `format` of the value (`int32`, `date-time`), when there is one.
    pub format: Option<String>,
    /// Whether the enclosing object requires the field.
    pub required: bool,
}

impl HoverDoc {
    /// The popup's Markdown: a header line (name, type, required), the description, the allowed
    /// values and the default, and the path.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut out = format!("**{}** · `{}`", self.name, self.type_label);
        if let Some(format) = &self.format {
            out.push_str(&format!(" · `{format}`"));
        }
        if self.required {
            out.push_str(" · required");
        }
        if let Some(description) = self.description.as_deref().filter(|d| !d.is_empty()) {
            out.push_str("\n\n");
            out.push_str(description);
        }
        if !self.enum_values.is_empty() {
            out.push_str("\n\nAllowed values: ");
            out.push_str(&code_list(self.enum_values.iter().cloned()));
        }
        if let Some(default) = &self.default {
            out.push_str(&format!("\n\nDefault: `{default}`"));
        }
        out.push_str(&format!("\n\n`{}`", self.path));
        out
    }
}

/// The docs of the field whose key or value is at `offset`, from the schema of the document's
/// kind (`schema_for` answers from what the caller holds; it does no I/O). `None` where the
/// schema says nothing: whitespace, a collection, an unknown field, an open object, a kind
/// without a schema.
pub fn hover_at(
    parsed: &ParseResult,
    schema_for: impl Fn(&Gvk) -> Option<Arc<JsonSchema>>,
    offset: usize,
) -> Option<HoverDoc> {
    let doc = parsed.doc_at(offset)?;
    let node = doc.node_at(offset)?;
    // Keys, values and list items that are scalars; not the collections around them.
    if !matches!(doc.node(node).kind, NodeKind::Scalar(_)) {
        return None;
    }
    let schema = schema_for(&document_gvk(parsed, doc)?)?;
    let path = doc.path_of(node, parsed.text());
    let Lookup::Known(field) = schema_at(&schema, &path) else {
        return None;
    };
    let parent = path_parent(&path);
    let required = match (path.segments().last(), schema_at(&schema, &parent)) {
        (Some(PathSegment::Key(key)), Lookup::Known(parent)) => {
            parent.required.iter().any(|r| **r == **key)
        }
        _ => false,
    };
    let name = match path.segments().last() {
        Some(PathSegment::Key(key)) => key.to_string(),
        Some(PathSegment::Index(n)) => format!("[{n}]"),
        None => return None,
    };
    Some(HoverDoc {
        range: doc.node(node).span.clone(),
        name,
        type_label: type_label(field),
        description: field.description.clone(),
        enum_values: field.enum_values.iter().map(enum_text).collect(),
        default: field.default.as_ref().map(enum_text),
        format: field.format.clone(),
        required,
        path,
    })
}

fn path_parent(path: &JsonPath) -> JsonPath {
    let segments = path.segments();
    JsonPath(segments[..segments.len().saturating_sub(1)].to_vec())
}
