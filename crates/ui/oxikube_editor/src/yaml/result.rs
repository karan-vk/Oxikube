//! `ParseResult`: the documents of one buffer plus its syntax diagnostics and buffer-wide lookups.

use std::ops::Range;
use std::sync::Arc;

use super::node::NodeId;
use super::path::DocPath;
use super::tree::DocTree;

/// A YAML syntax error at a byte range of the buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxDiagnostic {
    /// The document being parsed when the error was found.
    pub doc: usize,
    /// Bytes to underline: from the error position to the end of its line (empty at the end of
    /// the buffer).
    pub span: Range<usize>,
    /// The parser's message, e.g. `mapping values are not allowed in this context`.
    pub message: String,
}

/// The mapping key under an offset, as [`ParseResult::key_at`] reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyAt {
    /// The key's document and entry path.
    pub path: DocPath,
    /// The key node in that document.
    pub node: NodeId,
    /// The key's bytes.
    pub key_span: Range<usize>,
}

/// The spanned model of a whole buffer: one [`DocTree`] per document, the syntax diagnostics, and
/// the text they index (shared, so a cached result never disagrees with its text).
///
/// The result is immutable and `Send + Sync`: callers cache it as `Arc<ParseResult>` keyed by the
/// buffer version (see [`ParseCache`](super::ParseCache)) and reparse on edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseResult {
    pub(crate) text: Arc<str>,
    pub(crate) docs: Vec<DocTree>,
    pub(crate) diagnostics: Vec<SyntaxDiagnostic>,
}

impl ParseResult {
    /// The parsed text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The documents, in buffer order.
    #[must_use]
    pub fn docs(&self) -> &[DocTree] {
        &self.docs
    }

    /// The syntax errors, in buffer order. Empty when the text is valid YAML.
    #[must_use]
    pub fn diagnostics(&self) -> &[SyntaxDiagnostic] {
        &self.diagnostics
    }

    /// The document containing `offset`: the last one starting at or before it (the text between
    /// two documents belongs to the earlier one).
    #[must_use]
    pub fn doc_at(&self, offset: usize) -> Option<&DocTree> {
        let after = self.docs.partition_point(|d| d.span.start <= offset);
        self.docs.get(after.checked_sub(1)?)
    }

    /// The path of the deepest node at `offset`; on a key, the path of its entry.
    #[must_use]
    pub fn offset_to_path(&self, offset: usize) -> Option<DocPath> {
        let doc = self.doc_at(offset)?;
        let node = doc.node_at(offset)?;
        Some(DocPath {
            doc: doc.index,
            path: doc.path_of(node, &self.text),
        })
    }

    /// The span of the node at a path (see [`DocTree::lookup`]).
    #[must_use]
    pub fn path_to_span(&self, path: &DocPath) -> Option<Range<usize>> {
        let doc = self.docs.get(path.doc)?;
        let node = doc.lookup(&path.path, &self.text)?;
        Some(doc.node(node).span.clone())
    }

    /// The mapping key under `offset`, with its entry path.
    #[must_use]
    pub fn key_at(&self, offset: usize) -> Option<KeyAt> {
        let doc = self.doc_at(offset)?;
        let node = doc.key_at(offset)?;
        Some(KeyAt {
            path: DocPath {
                doc: doc.index,
                path: doc.path_of(node, &self.text),
            },
            node,
            key_span: doc.node(node).span.clone(),
        })
    }

    /// The decoded text of a scalar of document `doc` (see [`DocTree::scalar_value`]).
    #[must_use]
    pub fn scalar_value<'a>(&'a self, doc: &'a DocTree, id: NodeId) -> &'a str {
        doc.scalar_value(id, &self.text)
    }
}
