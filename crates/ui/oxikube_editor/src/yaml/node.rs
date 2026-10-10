//! Node types of the spanned YAML model: one flat, pre-order `Vec<Node>` per document.

use std::ops::Range;

/// Index of a [`Node`] inside its [`DocTree`](super::DocTree).
///
/// Ids are dense and in pre-order (source order), so a node's descendants are exactly the ids
/// between it and its subtree end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub(crate) u32);

impl NodeId {
    /// The position of this node in [`DocTree::nodes`](super::DocTree::nodes).
    #[must_use]
    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// The id of the node at `index` in [`DocTree::nodes`](super::DocTree::nodes).
    #[must_use]
    pub fn new(index: usize) -> Self {
        Self(u32::try_from(index).unwrap_or(u32::MAX))
    }
}

/// How a scalar was written in the source. Type decisions (string, int, bool, null) are the
/// validator's: the model keeps only the text and its notation (YAML 1.2: bare `yes` is a string).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScalarStyle {
    /// Unquoted. An empty plain scalar (`key:` with nothing after it) is an implicit null.
    Plain,
    /// `'single quoted'`.
    SingleQuoted,
    /// `"double quoted"`, with escapes.
    DoubleQuoted,
    /// `|` literal block.
    Literal,
    /// `>` folded block.
    Folded,
}

/// Block (indentation) or flow (`{}` / `[]`) notation of a collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CollectionStyle {
    /// Indentation-based `key: value` / `- item`.
    Block,
    /// Bracketed `{key: value}` / `[item]`.
    Flow,
}

/// What a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A mapping; its children alternate [`Role::Key`] and [`Role::Value`] nodes.
    Mapping(CollectionStyle),
    /// A sequence; its children are [`Role::Item`] nodes.
    Sequence(CollectionStyle),
    /// A scalar; its text is [`DocTree::scalar_value`](super::DocTree::scalar_value).
    Scalar(ScalarStyle),
    /// An alias (`*name`). `target` is the anchored node it refers to, when that node is in the
    /// same document and was parsed. Paths never go through an alias, and merge keys (`<<`) are
    /// not expanded: `<<` is an ordinary key whose value is the alias node.
    Alias {
        /// The node carrying the matching anchor, if known.
        target: Option<NodeId>,
    },
}

/// The place of a node in its parent, which is also its last JSON path segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The document's root node.
    Root,
    /// The key of a mapping entry. Its path is the entry's path (the same as its value's).
    Key,
    /// The value of a mapping entry whose key is `key` (the previous sibling).
    Value {
        /// The entry's key node.
        key: NodeId,
    },
    /// Item `index` of a sequence.
    Item {
        /// Zero-based position in the sequence.
        index: u32,
    },
}

/// One YAML node with its byte span in the whole buffer (not relative to its document).
///
/// Spans are on UTF-8 char boundaries. A block collection's span runs from its first key or `-`
/// to the end of its last child; a flow collection's from the opening to the closing bracket. A
/// quoted scalar's span includes its quotes. A block scalar's span is its content (the `|` / `>`
/// header line excluded), trailing blank lines trimmed. An implicit null has an empty span where
/// the value would be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// Byte range in the buffer.
    pub span: Range<usize>,
    /// The enclosing collection; `None` for the root.
    pub parent: Option<NodeId>,
    /// Position in the parent.
    pub role: Role,
    /// Kind and notation.
    pub kind: NodeKind,
    /// One past the last descendant's id.
    pub(crate) subtree_end: u32,
    /// The decoded scalar text, only when it differs from the source slice (quotes, escapes,
    /// folding); `None` means the value is `&text[span]`.
    pub(crate) decoded: Option<Box<str>>,
}

impl Node {
    /// Whether this is an empty plain scalar, i.e. an implicit null (`key:` with no value).
    #[must_use]
    pub fn is_implicit_null(&self) -> bool {
        matches!(self.kind, NodeKind::Scalar(ScalarStyle::Plain)) && self.span.is_empty()
    }
}
