//! `DocTree`: one YAML document as a flat pre-order node list, with path and offset lookups.

use std::ops::Range;

use super::node::{Node, NodeId, NodeKind, Role};
use super::path::{JsonPath, PathSegment};

/// One document of a buffer (the text between `---` markers), as a flat pre-order node list.
///
/// Lookups never re-walk the whole tree: offset → node is a binary search plus a walk up the
/// parents (O(log n + depth)), node → path walks the parents (O(depth)), and path → node walks
/// down (O(depth × width of the visited collections)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocTree {
    /// Zero-based position among the buffer's documents.
    pub index: usize,
    /// The document's bytes in the buffer: from its `---` (or first content) to its last node
    /// or its `...`.
    pub span: Range<usize>,
    /// Whether the document began with an explicit `---`.
    pub explicit_start: bool,
    pub(crate) nodes: Vec<Node>,
}

impl DocTree {
    /// All nodes in source (pre-)order; the root, if any, is first.
    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// The root node, `None` for an empty document (or one broken before its first node).
    #[must_use]
    pub fn root(&self) -> Option<NodeId> {
        (!self.nodes.is_empty()).then_some(NodeId(0))
    }

    /// The node with this id. Panics if the id belongs to another document.
    #[must_use]
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    /// The direct children of a node, in source order (keys and values alternate in a mapping).
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let end = self.node(id).subtree_end as usize;
        let mut next = id.index() + 1;
        std::iter::from_fn(move || {
            if next >= end {
                return None;
            }
            let child = next;
            next = (self.nodes[child].subtree_end as usize).max(child + 1);
            Some(NodeId::from_index(child))
        })
    }

    /// The `(key, value)` entries of a mapping; the value is `None` only for an entry cut off by
    /// a syntax error.
    pub fn entries(&self, id: NodeId) -> impl Iterator<Item = (NodeId, Option<NodeId>)> + '_ {
        let mut children = self.children(id).peekable();
        std::iter::from_fn(move || {
            let key = children.next()?;
            let value = children.next_if(|v| self.node(*v).role == Role::Value { key });
            Some((key, value))
        })
    }

    /// The text of a scalar (decoded: quotes and escapes removed, block scalars folded), or the
    /// source text of any other node (used for complex keys). `text` is the parsed buffer.
    #[must_use]
    pub fn scalar_value<'t>(&'t self, id: NodeId, text: &'t str) -> &'t str {
        let node = self.node(id);
        match &node.decoded {
            Some(decoded) => decoded,
            None => text.get(node.span.clone()).unwrap_or(""),
        }
    }

    /// The deepest node whose span contains `offset` (half-open: a node ending at `offset` does
    /// not contain it).
    #[must_use]
    pub fn node_at(&self, offset: usize) -> Option<NodeId> {
        // Pre-order ids are sorted by span start, so the last node starting at or before
        // `offset` is the deepest candidate; every node containing `offset` is one of its
        // ancestors (or itself).
        let after = self.nodes.partition_point(|n| n.span.start <= offset);
        let mut id = after.checked_sub(1).map(NodeId::from_index);
        while let Some(current) = id {
            let span = &self.node(current).span;
            if span.start <= offset && offset < span.end {
                return Some(current);
            }
            id = self.node(current).parent;
        }
        None
    }

    /// The mapping key under `offset` (for a collection used as a key, the key collection).
    #[must_use]
    pub fn key_at(&self, offset: usize) -> Option<NodeId> {
        let mut id = self.node_at(offset);
        while let Some(current) = id {
            if self.node(current).role == Role::Key {
                return Some(current);
            }
            id = self.node(current).parent;
        }
        None
    }

    /// The value node of the entry a key belongs to, if any.
    #[must_use]
    pub fn value_of_key(&self, key: NodeId) -> Option<NodeId> {
        let next = NodeId::from_index(self.node(key).subtree_end as usize);
        (next.index() < self.nodes.len() && self.node(next).role == Role::Value { key })
            .then_some(next)
    }

    /// The JSON path of a node. A key's path is its entry's path, the same as its value's (and so is
    /// the path of any node inside a collection used as a key).
    #[must_use]
    pub fn path_of(&self, id: NodeId, text: &str) -> JsonPath {
        let mut segments = Vec::new();
        let mut current = Some(id);
        while let Some(node_id) = current {
            let node = self.node(node_id);
            match node.role {
                Role::Root => break,
                Role::Key => {
                    // Inside a collection used as a key, every node has that entry's path.
                    segments.clear();
                    segments.push(PathSegment::Key(self.scalar_value(node_id, text).into()));
                }
                Role::Value { key } => {
                    segments.push(PathSegment::Key(self.scalar_value(key, text).into()));
                }
                Role::Item { index } => segments.push(PathSegment::Index(index as usize)),
            }
            current = node.parent;
        }
        segments.reverse();
        JsonPath(segments)
    }

    /// The node at a path: the entry's value for a key segment, or the key itself when a syntax
    /// error cut the value off. Duplicate keys resolve to the first. Aliases are not followed.
    #[must_use]
    pub fn lookup(&self, path: &JsonPath, text: &str) -> Option<NodeId> {
        let mut id = self.root()?;
        for segment in path.segments() {
            id = match (segment, self.node(id).kind) {
                (PathSegment::Key(want), NodeKind::Mapping(_)) => {
                    let (key, value) = self
                        .entries(id)
                        .find(|(key, _)| self.scalar_value(*key, text) == &**want)?;
                    value.unwrap_or(key)
                }
                (PathSegment::Index(want), NodeKind::Sequence(_)) => {
                    self.children(id).nth(*want)?
                }
                _ => return None,
            };
        }
        Some(id)
    }

    /// The span of a mapping entry's key, for a key node or a value node.
    #[must_use]
    pub fn key_span(&self, id: NodeId) -> Option<Range<usize>> {
        match self.node(id).role {
            Role::Key => Some(self.node(id).span.clone()),
            Role::Value { key } => Some(self.node(key).span.clone()),
            Role::Root | Role::Item { .. } => None,
        }
    }
}
