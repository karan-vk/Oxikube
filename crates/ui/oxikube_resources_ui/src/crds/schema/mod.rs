//! A CRD's `openAPIV3Schema` as a collapsible tree (E07-S07): what the Schema tab of a CRD's
//! detail shows. Plain Rust over the CRD's JSON, no GPUI.
//!
//! | File | Holds |
//! |---|---|
//! | `tree` | [`SchemaTree`]: which nodes are open, and [`SchemaTree::rows`], the visible rows |
//! | `node` | reading one schema node: its type text, description, enum, required, children |
//!
//! # Lazy and bounded
//!
//! Operator CRDs carry schemas of tens of thousands of nodes (an embedded pod template alone is
//! thousands). Nothing is built up front: the tree keeps only the set of open node keys, and
//! [`SchemaTree::rows`] walks the open nodes only, so its cost is the rows shown, not the size of
//! the schema. It runs when the user opens or closes a node, picks a version or the CRD changes,
//! never per frame. Two limits bound a pathological schema: [`MAX_DEPTH`] levels (the row below
//! says the rest is not shown) and [`MAX_ROWS`] rows (the last row says so).
//!
//! # What a row says
//!
//! The field name, its type as `kubectl explain` writes it (`[]string`, `map[string]string`,
//! `int-or-string`, `string (date-time)`), whether the parent requires it, the first paragraph of
//! its description, its enum values and its default. An array of objects, or a map of objects,
//! opens straight into the object's fields, as `kubectl explain` does, so `containers` shows
//! `image`, `name`, ... rather than a `[]` row in between.

mod node;
mod tree;

#[cfg(test)]
mod tests;

pub use tree::{
    MAX_DEPTH, MAX_ROWS, RowKind, SchemaRow, SchemaRows, SchemaTree, schema_root, version_names,
};
