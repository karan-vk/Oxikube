//! [`Value`]: the data of a document, read from the spanned YAML model.
//!
//! Both conversions go through this tree, and it is built from the model rather than from
//! `serde_json::Value` or a YAML crate's value type so that nothing is lost on the way: keys keep
//! the order of the source, and a number keeps the digits it was written with (an integer past
//! `u64` or a float past `f64` precision survives, which `serde_json` without
//! `arbitrary_precision` would round).
//!
//! Typing follows the YAML 1.2 core schema, as the schema validator (E10-S03) does: bare `yes`
//! is a string, `"123"` is a string, `123` an integer. Two readings follow the API server
//! instead, because converting must not change what a manifest means to the cluster: a
//! leading-zero integer (`0644`) is octal, and merge keys (`<<: *base`) are expanded.

use super::error::ConvertError;
use super::numbers::{float_text, int_text};
use super::warnings::Warnings;
use crate::validate::scalar::{ValueType, scalar_type};
use crate::yaml::{DocTree, NodeId, NodeKind, ParseResult, ScalarStyle};

/// The data of one document.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `null`, `~` or an empty value.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// An integer as decimal text (`-12`, `123456789012345678901234567890`).
    Int(String),
    /// A float as JSON-valid text (`0.5`, `1.5e3`).
    Float(String),
    /// A string.
    Str(String),
    /// A sequence.
    Seq(Vec<Value>),
    /// A mapping, in source order; keys are strings (JSON's rule).
    Map(Vec<(String, Value)>),
}

/// How deep documents may nest before reading stops (a stack guard; real manifests stay under 30).
const MAX_DEPTH: usize = 256;

/// How many values reading may build for one document. Aliases are expanded, so a few lines of
/// YAML can describe billions of values; past this the document is refused.
const MAX_VALUES: usize = 4_000_000;

/// Reads documents of one parse into [`Value`]s, counting what a conversion would lose.
pub(super) struct Reader<'a> {
    parsed: &'a ParseResult,
    doc: &'a DocTree,
    left: usize,
    pub(super) warnings: &'a mut Warnings,
}

impl<'a> Reader<'a> {
    pub(super) fn new(
        parsed: &'a ParseResult,
        doc: &'a DocTree,
        warnings: &'a mut Warnings,
    ) -> Self {
        Self {
            parsed,
            doc,
            left: MAX_VALUES,
            warnings,
        }
    }

    fn text(&self) -> &'a str {
        self.parsed.text()
    }

    /// The value of the document's root, `None` for an empty document.
    pub(super) fn document(&mut self) -> Result<Option<Value>, ConvertError> {
        match self.doc.root() {
            // `---` with nothing after it (or only comments) is no document.
            Some(root) if self.doc.node(root).is_implicit_null() => Ok(None),
            Some(root) => self.value(root, 0).map(Some),
            None => Ok(None),
        }
    }

    fn value(&mut self, id: NodeId, depth: usize) -> Result<Value, ConvertError> {
        let node = self.doc.node(id);
        let span = node.span.clone();
        if depth > MAX_DEPTH {
            return Err(ConvertError::at("nested too deeply to convert", span));
        }
        self.left = self.left.checked_sub(1).ok_or_else(|| {
            ConvertError::at(
                "the aliases expand to too many values (is this an alias bomb?)",
                span.clone(),
            )
        })?;
        let tag = self.tag_of(id);
        // Core tags that only restate the type are no loss; any other tag is.
        if let Some(tag) = tag
            && !matches!(
                tag,
                "!!str" | "!!int" | "!!float" | "!!bool" | "!!null" | "!!map" | "!!seq"
            )
        {
            self.warnings.tags += 1;
        }
        match node.kind {
            NodeKind::Scalar(style) => self.scalar(id, style, tag == Some("!!str")),
            NodeKind::Mapping(_) => self.mapping(id, depth),
            NodeKind::Sequence(_) => {
                let mut items = Vec::new();
                for child in self.doc.children(id) {
                    items.push(self.value(child, depth + 1)?);
                }
                Ok(Value::Seq(items))
            }
            NodeKind::Alias { target } => {
                let target = target.ok_or_else(|| {
                    ConvertError::at("this alias has no anchor in the document", span)
                })?;
                self.warnings.aliases += 1;
                self.value(target, depth + 1)
            }
        }
    }

    fn scalar(
        &mut self,
        id: NodeId,
        style: ScalarStyle,
        forced_string: bool,
    ) -> Result<Value, ConvertError> {
        let node = self.doc.node(id);
        let text = self.doc.scalar_value(id, self.text());
        if forced_string {
            return Ok(Value::Str(text.to_owned()));
        }
        Ok(match scalar_type(style, text) {
            ValueType::Null | ValueType::Object | ValueType::Array => Value::Null,
            ValueType::Bool => Value::Bool(text.eq_ignore_ascii_case("true")),
            ValueType::Int => {
                let mut octal = false;
                let digits = int_text(text, &mut octal)
                    .map_err(|e| ConvertError::at(e.message, node.span.clone()))?;
                self.warnings.octal += usize::from(octal);
                Value::Int(digits)
            }
            ValueType::Float => Value::Float(float_text(text).ok_or_else(|| {
                ConvertError::at(
                    format!("{text} has no JSON spelling (JSON has no infinity or NaN)"),
                    node.span.clone(),
                )
            })?),
            ValueType::String => {
                if style == ScalarStyle::Plain && is_yaml11_bool(text) {
                    self.warnings.bool_words += 1;
                }
                Value::Str(text.to_owned())
            }
        })
    }

    fn mapping(&mut self, id: NodeId, depth: usize) -> Result<Value, ConvertError> {
        let entries: Vec<(NodeId, Option<NodeId>)> = self.doc.entries(id).collect();
        // The keys written out, which a merged-in mapping must not override.
        let mut explicit: Vec<String> = Vec::new();
        for (key, _) in &entries {
            if !self.is_merge_key(*key) {
                explicit.push(self.key(*key)?);
            }
        }
        let mut out: Vec<(String, Value)> = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let Some(value) = value else {
                return Err(ConvertError::at(
                    "a key has no value",
                    self.doc.node(key).span.clone(),
                ));
            };
            if self.is_merge_key(key) {
                for merged in self.merge_sources(value, depth)? {
                    for (name, v) in merged {
                        if !explicit.contains(&name) && !out.iter().any(|(n, _)| *n == name) {
                            out.push((name, v));
                        }
                    }
                }
            } else {
                let name = self.key(key)?;
                out.push((name, self.value(value, depth + 1)?));
            }
        }
        Ok(Value::Map(out))
    }

    /// The tag written in front of the node `id`, if any: looked for in the text between the
    /// previous node (or the parent's opening, or the document's start) and this node.
    fn tag_of(&self, id: NodeId) -> Option<&'a str> {
        let nodes = self.doc.nodes();
        let node = &nodes[id.index()];
        let from = match id.index().checked_sub(1) {
            None => self.doc.span.start,
            Some(prev) if node.parent == Some(NodeId::new(prev)) => nodes[prev].span.start,
            Some(prev) => nodes[prev].span.end,
        };
        tag_in(self.text().get(from..node.span.start)?)
    }

    /// `<<` written as a plain key: the YAML merge key.
    fn is_merge_key(&self, key: NodeId) -> bool {
        matches!(
            self.doc.node(key).kind,
            NodeKind::Scalar(ScalarStyle::Plain)
        ) && self.doc.scalar_value(key, self.text()) == "<<"
    }

    /// The mappings a merge key's value names: one mapping (or alias to one), or a sequence of them.
    fn merge_sources(
        &mut self,
        value: NodeId,
        depth: usize,
    ) -> Result<Vec<Vec<(String, Value)>>, ConvertError> {
        let span = self.doc.node(value).span.clone();
        let nodes: Vec<NodeId> = match self.doc.node(value).kind {
            NodeKind::Sequence(_) => self.doc.children(value).collect(),
            _ => vec![value],
        };
        let mut sources = Vec::with_capacity(nodes.len());
        for node in nodes {
            match self.value(node, depth + 1)? {
                Value::Map(entries) => sources.push(entries),
                _ => {
                    return Err(ConvertError::at(
                        "a merge key (<<) needs a mapping, or a list of mappings",
                        span,
                    ));
                }
            }
        }
        Ok(sources)
    }

    /// A mapping key as JSON's string key: the scalar's text; `null` and booleans in their
    /// lowercase spelling; a collection or an alias cannot be a key.
    fn key(&self, key: NodeId) -> Result<String, ConvertError> {
        let node = self.doc.node(key);
        match node.kind {
            NodeKind::Scalar(style) => {
                let text = self.doc.scalar_value(key, self.text());
                Ok(match scalar_type(style, text) {
                    ValueType::Null if style == ScalarStyle::Plain => "null".to_owned(),
                    ValueType::Bool => text.to_ascii_lowercase(),
                    _ => text.to_owned(),
                })
            }
            _ => Err(ConvertError::at(
                "a key that is a list, a mapping or an alias has no JSON form",
                node.span.clone(),
            )),
        }
    }
}

/// The plain scalars YAML 1.1 (and so the API server's reader) takes for booleans although YAML
/// 1.2 does not.
fn is_yaml11_bool(text: &str) -> bool {
    matches!(
        text.to_ascii_lowercase().as_str(),
        "y" | "n" | "yes" | "no" | "on" | "off"
    )
}

/// The tag (`!!str`, `!local`) among the text between the end of the node before `text`'s node
/// and its start (anchors, `:`, `-`, brackets and comments are there too). Quoted strings cannot
/// be in that gap: they are nodes of their own.
fn tag_in(gap: &str) -> Option<&str> {
    let mut found = None;
    for line in gap.split('\n') {
        // A comment starts at a `#` that opens the line or follows white space.
        let code = line
            .char_indices()
            .find(|&(at, c)| {
                c == '#'
                    && line[..at]
                        .chars()
                        .next_back()
                        .is_none_or(char::is_whitespace)
            })
            .map_or(line, |(at, _)| &line[..at]);
        for token in
            code.split(|c: char| c.is_whitespace() || matches!(c, ',' | '[' | ']' | '{' | '}'))
        {
            if token.starts_with('!') {
                found = Some(token);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests;
