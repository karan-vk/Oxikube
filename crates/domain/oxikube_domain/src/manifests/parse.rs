//! [`parse_documents`]: manifest text to objects and per-document errors.

use std::fmt;

use serde_json::Value;

use super::source::SourceId;
use super::split::{Chunk, split};

/// One Kubernetes object read from a manifest.
#[derive(Clone, PartialEq)]
pub struct ManifestDoc {
    /// The text it was read from.
    pub source: SourceId,
    /// 1-based line in that text where the document starts (its first line with content; for an
    /// item of a `List`, the line of the list).
    pub line: usize,
    /// The object, as JSON: it converts straight to a `Resource` or an apply body.
    pub json: Value,
}

impl fmt::Debug for ManifestDoc {
    /// Names the object, never its content: it may be a Secret.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = |pointer: &str| self.json.pointer(pointer).and_then(Value::as_str);
        f.debug_struct("ManifestDoc")
            .field("source", &self.source)
            .field("line", &self.line)
            .field("kind", &text("/kind"))
            .field("name", &text("/metadata/name"))
            .finish_non_exhaustive()
    }
}

/// A document that could not be read. Carries a position and a generic message, never a piece of
/// the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    /// The text it was read from.
    pub source: SourceId,
    /// 1-based line: where the parser stopped when it says, else where the document starts.
    pub line: usize,
    /// 1-based column, when the parser gave one.
    pub column: Option<usize>,
    /// What is wrong, in words, without document content.
    pub message: String,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.source, self.line)?;
        if let Some(column) = self.column {
            write!(f, ":{column}")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// Reads every object in `text`. Never fails as a whole: a broken document is one
/// [`ManifestError`] and the rest still load. See the module docs for the rules.
///
/// Documents come back in text order. Cost is linear in the size of `text`; run it off the UI
/// thread for large input.
pub fn parse_documents(text: &str, source: SourceId) -> (Vec<ManifestDoc>, Vec<ManifestError>) {
    let mut docs = Vec::new();
    let mut errors = Vec::new();
    for chunk in split(text) {
        read_chunk(&chunk, &source, &mut docs, &mut errors);
    }
    (docs, errors)
}

fn read_chunk(
    chunk: &Chunk,
    source: &SourceId,
    docs: &mut Vec<ManifestDoc>,
    errors: &mut Vec<ManifestError>,
) {
    // Read as kubectl reads it (YAML 1.1): `defaultMode: 0644` is the octal integer 420.
    let options = serde_saphyr::options! { legacy_octal_numbers: true };
    let value: Value = match serde_saphyr::from_str_with_options(&chunk.text, options) {
        Ok(value) => value,
        Err(error) => {
            // The parser's message quotes the text, which may be a Secret: only the position
            // is kept.
            let (line, column) = error.location().map_or((chunk.content_line, None), |at| {
                let line = usize::try_from(at.line()).unwrap_or(1).max(1);
                let column = usize::try_from(at.column()).ok().filter(|c| *c > 0);
                (chunk.first_line + line - 1, column)
            });
            errors.push(ManifestError {
                source: source.clone(),
                line,
                column,
                message: "invalid YAML or JSON".into(),
            });
            return;
        }
    };
    let mut push_error = |message: &str| {
        errors.push(ManifestError {
            source: source.clone(),
            line: chunk.content_line,
            column: None,
            message: message.into(),
        });
    };
    match value {
        Value::Null => {}
        Value::Array(items) => {
            for item in items {
                accept(
                    item,
                    &mut |json| docs.push(doc(source, chunk, json)),
                    &mut push_error,
                );
            }
        }
        other => accept(
            other,
            &mut |json| docs.push(doc(source, chunk, json)),
            &mut push_error,
        ),
    }
}

fn doc(source: &SourceId, chunk: &Chunk, json: Value) -> ManifestDoc {
    ManifestDoc {
        source: source.clone(),
        line: chunk.content_line,
        json,
    }
}

/// Validates `value` as one object, or expands it when it is a `List`.
fn accept(value: Value, ok: &mut dyn FnMut(Value), fail: &mut dyn FnMut(&str)) {
    let Value::Object(mut map) = value else {
        fail("the document is not a mapping");
        return;
    };
    let is_list = map
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|k| k.ends_with("List"))
        && map.contains_key("items");
    if !is_list {
        accept_object(Value::Object(map), ok, fail);
        return;
    }
    match map.shift_remove("items") {
        Some(Value::Array(items)) => {
            for item in items {
                accept_object(item, ok, fail);
            }
        }
        Some(Value::Null) | None => {}
        Some(_) => fail("items of the list is not a sequence"),
    }
}

/// Validates one object's identity fields.
fn accept_object(value: Value, ok: &mut dyn FnMut(Value), fail: &mut dyn FnMut(&str)) {
    if !value.is_object() {
        fail("the document is not a mapping");
        return;
    }
    let text = |pointer: &str| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    };
    if !text("/apiVersion") {
        fail("the document has no apiVersion");
    } else if !text("/kind") {
        fail("the document has no kind");
    } else if !text("/metadata/name") {
        if text("/metadata/generateName") {
            fail("metadata.generateName is not supported by apply; set metadata.name");
        } else {
            fail("the document has no metadata.name");
        }
    } else {
        ok(value);
    }
}
