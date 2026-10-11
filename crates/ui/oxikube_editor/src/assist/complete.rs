//! Completion: sibling fields, enum and boolean values, and whole blocks, from the schema.
//!
//! The line being typed is rarely valid YAML (`ima`), so the spanned model of the buffer cannot
//! place it. [`complete_at`] reads the line lexically (`line`), then asks the model about a copy
//! of the text where that line is a complete `key:`: the key's parent mapping gives the path, the
//! path gives the schema (`lookup`), and the mapping's other keys say what is already present.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::{AdditionalProperties, JsonSchema, SchemaType};

use super::buffer::Buffer;
use super::describe::{field_docs, type_label};
use super::line::{KeySite, Site, site_at};
use super::lookup::{Lookup, schema_at};
use super::snippets::{self, indented};
use super::values::complete_value;
use crate::validate::document_gvk;
use crate::yaml::{NodeKind, ParseResult, PathSegment, parse};

/// What a [`CompletionItem`] inserts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompletionKind {
    /// A field name (`image: `).
    Field,
    /// A value of the field being edited.
    Value,
    /// A block of lines (a whole container).
    Snippet,
}

/// One suggestion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    /// What the menu shows and the typed word filters.
    pub label: String,
    /// What it inserts.
    pub kind: CompletionKind,
    /// A short note: the type, and "required".
    pub detail: Option<String>,
    /// The schema's description and values, as Markdown.
    pub documentation: Option<String>,
    /// The text that replaces [`replace`](Self::replace), verbatim.
    pub insert_text: String,
    /// The bytes replaced: the word (or value) being typed.
    pub replace: Range<usize>,
    /// Whether the field is `required` by its object (listed first).
    pub required: bool,
}

/// The key of a mapping that the sentinel replaces when nothing has been typed yet.
const PLACEHOLDER: &str = "x";

/// Whether anything can be offered at `offset` of `text`: false in a comment, a flow collection,
/// a block scalar and the like. Cheap (the cursor's line and a few above), so callers ask before
/// parsing.
#[must_use]
pub fn can_complete(text: &str, offset: usize) -> bool {
    site_at(text, offset).is_some()
}

/// The completions at the cursor `offset` of `buffer`'s text, best first.
///
/// * **Key position** (a half-typed key, a blank line, after `- `): the properties of the
///   enclosing object that are not already present, required ones first, filtered by the typed
///   word; plus the blocks of `snippets` for lists the object can hold.
/// * **Value position** (`restartPolicy: `): the field's `enum` members, or `true` / `false`.
/// * Nothing in a comment or block scalar, where the schema is open (a free-form object,
///   `x-kubernetes-preserve-unknown-fields`, a `additionalProperties` map), or for a kind
///   without a schema.
///
/// `schema_for` answers from what the caller holds; it does no I/O.
#[must_use]
pub fn complete_at(
    buffer: &Buffer,
    schema_for: impl Fn(&Gvk) -> Option<Arc<JsonSchema>>,
    offset: usize,
) -> Vec<CompletionItem> {
    match site_at(buffer.text(), offset) {
        Some(Site::Key(site)) => complete_key(buffer, &schema_for, &site),
        Some(Site::Value(site)) => complete_value(buffer.parse(), &schema_for, &site),
        None => Vec::new(),
    }
}

fn complete_key(
    buffer: &Buffer,
    schema_for: &dyn Fn(&Gvk) -> Option<Arc<JsonSchema>>,
    site: &KeySite,
) -> Vec<CompletionItem> {
    let text = buffer.text();
    let prefix = site.prefix(text);
    let mut patched = String::with_capacity(text.len() + 8);
    patched.push_str(&text[..site.key_start]);
    patched.push_str(if prefix.is_empty() {
        PLACEHOLDER
    } else {
        prefix
    });
    patched.push(':');
    patched.push_str(&text[site.line_end..]);
    let reparsed = (patched != text).then(|| parse(&patched));
    let tree: &ParseResult = match &reparsed {
        Some(patched) => patched,
        None => buffer.parse(),
    };
    let Some(doc) = tree.doc_at(site.key_start) else {
        return Vec::new();
    };
    let Some(key) = doc.key_at(site.key_start) else {
        return Vec::new();
    };
    let Some(mapping) = doc.node(key).parent else {
        return Vec::new();
    };
    if doc.node(key).span.start != site.key_start
        || !matches!(doc.node(mapping).kind, NodeKind::Mapping(_))
    {
        return Vec::new();
    }
    let container = doc.path_of(mapping, tree.text());
    let at_root = container.segments().is_empty();
    let Some(schema) = document_gvk(tree, doc).and_then(|gvk| schema_for(&gvk)) else {
        return basics(tree, site, at_root);
    };
    let Lookup::Known(object) = schema_at(&schema, &container) else {
        return Vec::new();
    };
    let present: HashSet<&str> = doc
        .entries(mapping)
        .filter(|(k, _)| *k != key)
        .map(|(k, _)| doc.scalar_value(k, tree.text()))
        .collect();
    let alone = doc.entries(mapping).count() == 1;
    let mut fields = Vec::new();
    let mut blocks = Vec::new();
    for (name, property) in object.properties.iter() {
        if present.contains(name) || (at_root && name == "status") {
            continue;
        }
        if let Some(rank) = rank(prefix, name) {
            let required = object.required.iter().any(|r| r == name);
            fields.push((rank, field_item(name, property, required, site)));
        }
        if !site.colon_follows {
            blocks.extend(snippets::for_list(name).filter_map(|s| {
                let matched = rank(prefix, s.label).or_else(|| rank(prefix, name))?;
                let child = " ".repeat(site.column + 2);
                let body = indented(s.body, site.column + 4);
                Some((
                    matched,
                    block_item(s, format!("{name}:\n{child}- {body}"), site),
                ))
            }));
        }
    }
    // Items of a list the schema has snippets for: the whole block, when the item is empty.
    let mut items = Vec::new();
    if let [.., PathSegment::Key(list), PathSegment::Index(_)] = container.segments()
        && site.item_start
        && alone
        && !site.colon_follows
    {
        items.extend(snippets::for_list(list).filter_map(|s| {
            rank(prefix, s.label)?;
            Some(block_item(s, indented(s.body, site.column), site))
        }));
    }
    fields.sort_by(|a, b| (a.0, !a.1.required, &a.1.label).cmp(&(b.0, !b.1.required, &b.1.label)));
    // Stable: blocks of one rank stay in the table's order (the plainest first).
    blocks.sort_by_key(|block| block.0);
    items.extend(fields.into_iter().map(|(_, item)| item));
    items.extend(blocks.into_iter().map(|(_, item)| item));
    items
}

/// A kind without a known schema still has `apiVersion` and `kind`, once the root is being typed.
fn basics(tree: &ParseResult, site: &KeySite, at_root: bool) -> Vec<CompletionItem> {
    if !at_root {
        return Vec::new();
    }
    let text = tree.text();
    let prefix = site.prefix(text);
    let Some(doc) = tree.doc_at(site.key_start) else {
        return Vec::new();
    };
    let present: HashSet<&str> = doc
        .root()
        .into_iter()
        .flat_map(|root| doc.entries(root))
        .map(|(k, _)| doc.scalar_value(k, text))
        .collect();
    [
        (
            "apiVersion",
            "The versioned schema of this object, e.g. `v1` or `apps/v1`.",
        ),
        ("kind", "The kind of object, e.g. `Pod`."),
    ]
    .into_iter()
    .filter(|(name, _)| !present.contains(name) && rank(prefix, name).is_some())
    .map(|(name, docs)| CompletionItem {
        label: name.to_owned(),
        kind: CompletionKind::Field,
        detail: Some("string · required".to_owned()),
        documentation: Some(docs.to_owned()),
        insert_text: insert_for(name, Shape::Scalar, site),
        replace: site.key_start..site.replace_end,
        required: true,
    })
    .collect()
}

/// How well `word` matches `name`: 0 for a prefix, 1 for a substring (both ignoring case),
/// `None` otherwise. An empty word matches everything as a prefix.
pub(super) fn rank(word: &str, name: &str) -> Option<u8> {
    let (word, name) = (word.to_lowercase(), name.to_lowercase());
    if name.starts_with(&word) {
        Some(0)
    } else {
        name.contains(&word).then_some(1)
    }
}

/// How a field's value is laid out when its key is inserted.
#[derive(Clone, Copy)]
enum Shape {
    Scalar,
    Object,
    Array,
}

fn shape(schema: &JsonSchema) -> Shape {
    let has = |t: SchemaType| schema.types.contains(&t);
    if has(SchemaType::Array) || (schema.types.is_empty() && schema.items.is_some()) {
        Shape::Array
    } else if has(SchemaType::Object)
        || (schema.types.is_empty()
            && (!schema.properties.is_empty()
                || matches!(
                    schema.additional_properties,
                    AdditionalProperties::Schema(_)
                )))
    {
        Shape::Object
    } else {
        Shape::Scalar
    }
}

/// `name: `, or `name:` and an indented line for an object or a list; just `name` when a colon
/// already follows the word.
fn insert_for(name: &str, shape: Shape, site: &KeySite) -> String {
    if site.colon_follows {
        return name.to_owned();
    }
    let child = " ".repeat(site.column + 2);
    match shape {
        Shape::Scalar => format!("{name}: "),
        Shape::Object => format!("{name}:\n{child}"),
        Shape::Array => format!("{name}:\n{child}- "),
    }
}

fn field_item(name: &str, property: &JsonSchema, required: bool, site: &KeySite) -> CompletionItem {
    let ty = type_label(property);
    CompletionItem {
        label: name.to_owned(),
        kind: CompletionKind::Field,
        detail: Some(if required {
            format!("{ty} · required")
        } else {
            ty
        }),
        documentation: field_docs(property),
        insert_text: insert_for(name, shape(property), site),
        replace: site.key_start..site.replace_end,
        required,
    }
}

fn block_item(snippet: &snippets::Snippet, insert_text: String, site: &KeySite) -> CompletionItem {
    CompletionItem {
        label: snippet.label.to_owned(),
        kind: CompletionKind::Snippet,
        detail: Some(snippet.detail.to_owned()),
        documentation: Some(format!("```yaml\n{}\n```", snippet.body)),
        insert_text,
        replace: site.key_start..site.replace_end,
        required: false,
    }
}
