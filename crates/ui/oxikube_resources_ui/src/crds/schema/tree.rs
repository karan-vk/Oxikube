//! [`SchemaTree`]: which nodes of a schema are open, and the rows that follow from it.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::Value;

use super::node;

/// How many levels deep a row can be. A deeper node is replaced by a [`RowKind::DepthLimit`] row.
pub const MAX_DEPTH: usize = 12;
/// How many rows one walk produces. The row after the last says how many were left out.
pub const MAX_ROWS: usize = 2000;

/// What a [`SchemaRow`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A field of an object.
    Field,
    /// Stands in for the levels below [`MAX_DEPTH`].
    DepthLimit,
    /// Stands in for the rows beyond [`MAX_ROWS`]; `name` says how many.
    Truncated,
}

/// One visible row of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaRow {
    /// The node's path (`spec.template.containers.image`), unique in the tree; open and closed
    /// state is kept by it.
    pub key: Arc<str>,
    /// Zero for the top-level fields.
    pub depth: usize,
    /// What the row is.
    pub kind: RowKind,
    /// The field name (or the note of a limit row).
    pub name: String,
    /// The type, as `kubectl explain` writes it.
    pub ty: String,
    /// Whether the parent object requires the field.
    pub required: bool,
    /// The first paragraph of the description, shortened.
    pub description: Option<String>,
    /// The allowed values.
    pub enum_values: Vec<String>,
    /// The default.
    pub default: Option<String>,
    /// Whether the node has fields to show.
    pub expandable: bool,
    /// Whether they are shown now.
    pub open: bool,
}

/// The rows of one walk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchemaRows {
    /// The visible rows, depth first.
    pub rows: Vec<SchemaRow>,
    /// Whether the walk stopped at [`MAX_ROWS`].
    pub truncated: bool,
}

/// The `openAPIV3Schema` of `version` in the CRD `crd`, if it declares one.
pub fn schema_root<'a>(crd: &'a Value, version: &str) -> Option<&'a Value> {
    crd.pointer("/spec/versions")?
        .as_array()?
        .iter()
        .find(|v| v.get("name").and_then(Value::as_str) == Some(version))?
        .pointer("/schema/openAPIV3Schema")
}

/// The names of the versions of `crd` that declare a schema, in the CRD's order.
pub fn version_names(crd: &Value) -> Vec<String> {
    crd.pointer("/spec/versions")
        .and_then(Value::as_array)
        .map(|versions| {
            versions
                .iter()
                .filter(|v| v.pointer("/schema/openAPIV3Schema").is_some())
                .filter_map(|v| v.get("name").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The open nodes of one schema. Built empty (every field of the top level shown, nothing below
/// it open) and changed by [`toggle`](Self::toggle); the rows are read with
/// [`rows`](Self::rows) over the schema as it is now, so the state survives the CRD changing
/// under it. See the [module docs](super).
#[derive(Debug, Clone, Default)]
pub struct SchemaTree {
    open: HashSet<Arc<str>>,
}

impl SchemaTree {
    /// A tree with nothing open.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the node `key` is open.
    pub fn is_open(&self, key: &str) -> bool {
        self.open.contains(key)
    }

    /// Opens the node `key` when it is closed, closes it when it is open; whether it is open now.
    /// (A key that names no node is harmless: no row reads it.)
    pub fn toggle(&mut self, key: &str) -> bool {
        if self.open.remove(key) {
            return false;
        }
        self.open.insert(key.into());
        true
    }

    /// Opens or closes `key` explicitly. `true` when that changed it.
    pub fn set_open(&mut self, key: &str, open: bool) -> bool {
        if open {
            self.open.insert(key.into())
        } else {
            self.open.remove(key)
        }
    }

    /// Closes everything.
    pub fn collapse_all(&mut self) {
        self.open.clear();
    }

    /// The visible rows of `root` (the schema's root node): its fields, and below each open one
    /// its own, down to [`MAX_DEPTH`] and up to [`MAX_ROWS`].
    pub fn rows(&self, root: &Value) -> SchemaRows {
        let mut out = SchemaRows::default();
        self.walk(root, "", 0, &mut out);
        if out.truncated {
            out.rows.push(note(
                "",
                0,
                RowKind::Truncated,
                format!("…more fields are not shown (the first {MAX_ROWS} rows are)"),
            ));
        }
        out
    }

    /// Adds the fields of `parent` (at `depth`, under `path`); `false` once the row limit is hit.
    fn walk(&self, parent: &Value, path: &str, depth: usize, out: &mut SchemaRows) -> bool {
        for (name, schema, required) in node::fields(parent) {
            if out.rows.len() >= MAX_ROWS {
                out.truncated = true;
                return false;
            }
            let key: Arc<str> = if path.is_empty() {
                name.into()
            } else {
                format!("{path}.{name}").into()
            };
            let expandable = node::container(schema).is_some();
            let open = expandable && depth + 1 < MAX_DEPTH && self.open.contains(&key);
            out.rows.push(SchemaRow {
                key: key.clone(),
                depth,
                kind: RowKind::Field,
                name: name.to_owned(),
                ty: node::type_text(schema),
                required,
                description: node::description(schema),
                enum_values: node::enum_values(schema),
                default: node::default_text(schema),
                expandable,
                open,
            });
            if expandable && self.open.contains(&key) {
                if depth + 1 >= MAX_DEPTH {
                    out.rows.push(note(
                        &key,
                        depth + 1,
                        RowKind::DepthLimit,
                        "…deeper levels are not shown".to_owned(),
                    ));
                } else if !self.walk(schema, &key, depth + 1, out) {
                    return false;
                }
            }
        }
        true
    }
}

fn note(parent: &str, depth: usize, kind: RowKind, text: String) -> SchemaRow {
    SchemaRow {
        key: format!("{parent}#{kind:?}").into(),
        depth,
        kind,
        name: text,
        ty: String::new(),
        required: false,
        description: None,
        enum_values: Vec::new(),
        default: None,
        expandable: false,
        open: false,
    }
}
