//! Turns a stream of parser events (already in buffer byte offsets) into [`DocTree`]s.
//!
//! Parser-agnostic: `parse.rs` maps granit-parser events onto these calls, so a parser swap
//! stays there. The builder also implements the re-attach step of error recovery: after a syntax
//! error it can keep a collection open and continue it from a later parser run.

use std::collections::HashMap;
use std::ops::Range;

use super::node::{CollectionStyle, Node, NodeId, NodeKind, Role};
use super::tree::DocTree;

/// Mapping or sequence, for the open-collection stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    Mapping,
    Sequence,
}

/// One open collection.
#[derive(Debug)]
struct Frame {
    id: NodeId,
    shape: Shape,
    style: CollectionStyle,
    /// A mapping's key still waiting for its value.
    pending_key: Option<NodeId>,
    next_index: u32,
    /// Furthest end of the collection's children so far.
    end: usize,
}

/// What the next parser run continues (set by [`Builder::attach`] / [`Builder::adopt`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Attach {
    /// The run's first root node continues the open collection on top of the stack.
    Continue(Shape),
    /// The run's first root node becomes the root of the current (empty) document.
    Adopt,
}

/// An open block collection, as error recovery sees it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OpenBlock {
    /// Position in the open-collection stack.
    pub depth: usize,
    pub shape: Shape,
    /// Byte offset where the collection starts (its first key or `-`).
    pub start: usize,
}

pub(crate) struct Builder {
    docs: Vec<DocTree>,
    doc: Option<DocTree>,
    frames: Vec<Frame>,
    anchors: HashMap<usize, NodeId>,
    attach: Option<Attach>,
    /// Depth of a subtree being dropped (a run's root that cannot be re-attached).
    skip_depth: usize,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            docs: Vec::new(),
            doc: None,
            frames: Vec::new(),
            anchors: HashMap::new(),
            attach: None,
            skip_depth: 0,
        }
    }

    pub fn doc_start(&mut self, range: Range<usize>, explicit: bool) {
        if self.attach.is_some() && self.doc.is_some() {
            return;
        }
        self.finish_doc(range.start);
        self.doc = Some(DocTree {
            index: self.docs.len(),
            span: range,
            explicit_start: explicit,
            nodes: Vec::new(),
        });
    }

    pub fn doc_end(&mut self, range: Range<usize>) {
        self.attach = None;
        self.skip_depth = 0;
        self.finish_doc(range.end);
    }

    pub fn collection_start(
        &mut self,
        shape: Shape,
        style: CollectionStyle,
        range: Range<usize>,
        anchor: usize,
    ) {
        if self.skip_depth > 0 {
            self.skip_depth += 1;
            return;
        }
        match self.attach.take() {
            Some(Attach::Continue(want)) if want == shape && style == CollectionStyle::Block => {
                return;
            }
            Some(Attach::Continue(_)) => {
                self.skip_depth = 1;
                return;
            }
            Some(Attach::Adopt) | None => {}
        }
        let kind = match shape {
            Shape::Mapping => NodeKind::Mapping(style),
            Shape::Sequence => NodeKind::Sequence(style),
        };
        let end = range.end;
        if let Some(id) = self.push(kind, range, None, anchor) {
            self.frames.push(Frame {
                id,
                shape,
                style,
                pending_key: None,
                next_index: 0,
                end,
            });
        } else {
            self.skip_depth = 1;
        }
    }

    pub fn collection_end(&mut self, range: Range<usize>) {
        if self.skip_depth > 0 {
            self.skip_depth -= 1;
            return;
        }
        self.close_top(Some(range.end));
    }

    pub fn leaf(
        &mut self,
        kind: NodeKind,
        range: Range<usize>,
        decoded: Option<Box<str>>,
        anchor: usize,
    ) {
        if self.skip_depth > 0 {
            return;
        }
        if let Some(Attach::Continue(_)) = self.attach.take() {
            return;
        }
        self.push(kind, range, decoded, anchor);
    }

    /// The node an alias id refers to in the current document.
    pub fn anchor(&self, id: usize) -> Option<NodeId> {
        self.anchors.get(&id).copied()
    }

    /// Whether the next node is a block mapping's value (its key has been seen).
    pub fn awaits_value(&self) -> bool {
        self.frames.last().is_some_and(|f| {
            f.shape == Shape::Mapping
                && f.style == CollectionStyle::Block
                && f.pending_key.is_some()
        })
    }

    /// The open block collections, outermost first.
    pub fn open_blocks(&self) -> impl Iterator<Item = OpenBlock> + '_ {
        let doc = self.doc.as_ref();
        self.frames
            .iter()
            .enumerate()
            .filter(|(_, f)| f.style == CollectionStyle::Block)
            .filter_map(move |(depth, f)| {
                Some(OpenBlock {
                    depth,
                    shape: f.shape,
                    start: doc?.nodes.get(f.id.index())?.span.start,
                })
            })
    }

    /// Index of the document being built (the next one's when none is open).
    pub fn doc_index(&self) -> usize {
        self.doc.as_ref().map_or(self.docs.len(), |d| d.index)
    }

    /// Whether the current document has no node yet.
    pub fn doc_is_empty(&self) -> bool {
        self.doc.as_ref().is_none_or(|d| d.nodes.is_empty())
    }

    /// Recovery: close every collection deeper than `depth` and let the next run's first root
    /// collection continue the one at `depth` (its entries or items are appended to it).
    pub fn attach(&mut self, depth: usize) {
        while self.frames.len() > depth + 1 {
            self.close_top(None);
        }
        if let Some(top) = self.frames.last_mut() {
            top.pending_key = None;
            self.attach = Some(Attach::Continue(top.shape));
        }
        self.skip_depth = 0;
    }

    /// Forgets any re-attach the run just ended did not use.
    pub fn end_run(&mut self) {
        self.attach = None;
        self.skip_depth = 0;
    }

    /// Recovery: the next run's first document fills the current, still empty, document.
    pub fn adopt(&mut self) {
        while !self.frames.is_empty() {
            self.close_top(None);
        }
        self.attach = Some(Attach::Adopt);
        self.skip_depth = 0;
    }

    /// Recovery: close the current document; the next run starts new ones.
    pub fn abandon_doc(&mut self, end: usize) {
        self.attach = None;
        self.skip_depth = 0;
        self.finish_doc(end);
    }

    pub fn finish(mut self, end: usize) -> Vec<DocTree> {
        self.finish_doc(end);
        self.docs
    }

    fn push(
        &mut self,
        kind: NodeKind,
        span: Range<usize>,
        decoded: Option<Box<str>>,
        anchor: usize,
    ) -> Option<NodeId> {
        if self.doc.is_none() {
            self.doc_start(span.start..span.start, false);
        }
        let doc = self.doc.as_mut()?;
        let id = NodeId::new(doc.nodes.len());
        let (parent, role) = match self.frames.last_mut() {
            None if doc.nodes.is_empty() => (None, Role::Root),
            None => return None,
            Some(frame) => {
                let role = match frame.shape {
                    Shape::Sequence => {
                        frame.next_index += 1;
                        Role::Item {
                            index: frame.next_index - 1,
                        }
                    }
                    Shape::Mapping => match frame.pending_key.take() {
                        Some(key) => Role::Value { key },
                        None => {
                            frame.pending_key = Some(id);
                            Role::Key
                        }
                    },
                };
                frame.end = frame.end.max(span.end);
                (Some(frame.id), role)
            }
        };
        if anchor != 0 {
            self.anchors.insert(anchor, id);
        }
        doc.nodes.push(Node {
            span,
            parent,
            role,
            kind,
            subtree_end: id.0 + 1,
            decoded,
        });
        Some(id)
    }

    /// Pops the innermost open collection; a flow collection ends at `flow_end` when its closing
    /// bracket was seen, any other at the end of its last child.
    fn close_top(&mut self, flow_end: Option<usize>) {
        let Some(frame) = self.frames.pop() else {
            return;
        };
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        let subtree_end = u32::try_from(doc.nodes.len()).unwrap_or(u32::MAX);
        let Some(node) = doc.nodes.get_mut(frame.id.index()) else {
            return;
        };
        let end = match (frame.style, flow_end) {
            (CollectionStyle::Flow, Some(end)) => end,
            _ => frame.end,
        };
        node.span.end = end.max(node.span.start);
        node.subtree_end = subtree_end;
        let end = node.span.end;
        if let Some(parent) = self.frames.last_mut() {
            parent.end = parent.end.max(end);
        }
    }

    fn finish_doc(&mut self, end: usize) {
        while !self.frames.is_empty() {
            self.close_top(None);
        }
        self.anchors.clear();
        if let Some(mut doc) = self.doc.take() {
            let nodes_end = doc.nodes.first().map_or(0, |root| root.span.end);
            doc.span.end = doc.span.end.max(nodes_end).max(end);
            self.docs.push(doc);
        }
    }
}
