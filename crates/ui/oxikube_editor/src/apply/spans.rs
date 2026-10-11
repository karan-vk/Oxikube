//! A server's rejection mapped onto the buffer: each `Status.details.causes[].field`
//! (`spec.template.spec.containers[0].image`, `.spec.containers[name="web"].image` for a
//! server-side apply) becomes an error on the line that holds it.
//!
//! The server names fields in its own notation: dots between keys, `[n]` for an index, `[k]` for
//! a map key with dots or slashes in it (`metadata.labels[app.kubernetes.io/name]`), and for a
//! server-side apply `[name="web"]` for the list item whose `name` is `web`. A field the buffer
//! does not have (a "Required value") is put on the nearest part of its path the buffer does
//! have, and says so. A cause without a field, or one that lands on nothing, is returned as text
//! for the problems line.

use oxikube_domain::FieldCause;
use oxikube_ui::editor::{DiagnosticLevel, EditorDiagnostic};

use crate::yaml::{DocTree, JsonPath, NodeId, NodeKind, ParseResult, Role, parse};

/// The code of a diagnostic the server reported.
pub const SERVER_CODE: &str = "server";

/// One step of a server field path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// A mapping key.
    Key(String),
    /// A list index.
    Index(usize),
    /// The list item whose fields have these values (`[name="web"]`).
    Item(Vec<(String, String)>),
}

/// Where the server's causes go.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlacedCauses {
    /// One error per cause the buffer has a place for.
    pub diagnostics: Vec<EditorDiagnostic>,
    /// "field: message" for the causes it has none for.
    pub unplaced: Vec<String>,
}

/// Puts each of `causes` on the part of `text` (the buffer, one object) its field names.
/// Parses `text`: run it off the UI thread.
pub fn place_causes(text: &str, causes: &[FieldCause]) -> PlacedCauses {
    let parsed = parse(text);
    let mut placed = PlacedCauses::default();
    for cause in causes {
        let line = describe(cause);
        let spot = (!cause.field.is_empty())
            .then(|| parse_field(&cause.field))
            .flatten()
            .and_then(|steps| locate(&parsed, &steps));
        match spot {
            Some((range, exact)) => {
                let message = if exact {
                    line
                } else {
                    format!("{line} (not in the manifest)")
                };
                placed.diagnostics.push(
                    EditorDiagnostic::new(range, DiagnosticLevel::Error, message)
                        .with_code(SERVER_CODE),
                );
            }
            None => placed.unplaced.push(line),
        }
    }
    placed.diagnostics.sort_by_key(|d| d.range.start);
    placed
}

/// "spec.replicas: Invalid value", or the message alone without a field.
fn describe(cause: &FieldCause) -> String {
    let message = if cause.message.is_empty() {
        cause.reason.as_str()
    } else {
        cause.message.as_str()
    };
    let field = cause.field.trim_start_matches('.');
    if field.is_empty() {
        message.to_owned()
    } else {
        format!("{field}: {message}")
    }
}

/// The steps of a server field path; `None` when it is not one.
pub(crate) fn parse_field(field: &str) -> Option<Vec<Step>> {
    let mut steps = Vec::new();
    let mut rest = field.trim().trim_start_matches('.');
    while !rest.is_empty() {
        if let Some(inner) = rest.strip_prefix('[') {
            let end = closing_bracket(inner)?;
            steps.push(bracket(&inner[..end]));
            rest = &inner[end + 1..];
        } else {
            let end = rest.find(['.', '[']).unwrap_or(rest.len());
            if end == 0 {
                return None;
            }
            steps.push(Step::Key(rest[..end].to_owned()));
            rest = &rest[end..];
        }
        rest = rest.strip_prefix('.').unwrap_or(rest);
    }
    Some(steps)
}

/// The index of the `]` that closes a bracket, skipping quoted text.
fn closing_bracket(inner: &str) -> Option<usize> {
    let mut quoted = false;
    let mut escaped = false;
    for (i, c) in inner.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ']' if !quoted => return Some(i),
            _ => {}
        }
    }
    None
}

/// `0` is an index, `name="web",port=80` an item selector, anything else a key.
fn bracket(inner: &str) -> Step {
    if let Ok(index) = inner.parse::<usize>() {
        return Step::Index(index);
    }
    let unquote = |s: &str| {
        let s = s.trim();
        s.strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .map_or_else(|| s.to_owned(), |s| s.replace("\\\"", "\""))
    };
    let pairs: Option<Vec<(String, String)>> = split_pairs(inner)
        .into_iter()
        .map(|pair| {
            let (key, value) = pair.split_once('=')?;
            let key = key.trim();
            (!key.is_empty() && !key.contains(['"', '/'])).then(|| (key.to_owned(), unquote(value)))
        })
        .collect();
    match pairs {
        Some(pairs) if !pairs.is_empty() => Step::Item(pairs),
        _ => Step::Key(unquote(inner)),
    }
}

/// `a="x,y",b=2` split at the commas outside quotes.
fn split_pairs(inner: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (i, c) in inner.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                parts.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    parts
}

/// The range to mark for `steps` in the first document, and whether the whole path was found.
fn locate(parsed: &ParseResult, steps: &[Step]) -> Option<(std::ops::Range<usize>, bool)> {
    let doc = parsed.docs().first()?;
    let text = parsed.text();
    let mut path = JsonPath::root();
    let mut found: Option<NodeId> = None;
    let mut exact = true;
    for step in steps {
        let next = match step {
            Step::Key(key) => path.clone().key(key.as_str()),
            Step::Index(index) => path.clone().index(*index),
            Step::Item(fields) => match item_index(doc, text, &path, fields) {
                Some(index) => path.clone().index(index),
                None => {
                    exact = false;
                    break;
                }
            },
        };
        match doc.lookup(&next, text) {
            Some(node) => {
                found = Some(node);
                path = next;
            }
            None => {
                exact = false;
                break;
            }
        }
    }
    let node = found?;
    Some((mark(doc, node), exact))
}

/// The index of the item of the list at `path` whose `fields` have the given values.
fn item_index(
    doc: &DocTree,
    text: &str,
    path: &JsonPath,
    fields: &[(String, String)],
) -> Option<usize> {
    let list = doc.lookup(path, text)?;
    if !matches!(doc.node(list).kind, NodeKind::Sequence(_)) {
        return None;
    }
    (0..doc.children(list).count()).find(|index| {
        let item = path.clone().index(*index);
        fields.iter().all(|(key, value)| {
            doc.lookup(&item.clone().key(key.as_str()), text)
                .is_some_and(|node| doc.scalar_value(node, text) == value)
        })
    })
}

/// What to underline for `node`: the key of a mapping entry (a whole block value would underline
/// many lines), the first key of a list item that is a mapping, else the node.
fn mark(doc: &DocTree, node: NodeId) -> std::ops::Range<usize> {
    let n = doc.node(node);
    match n.role {
        Role::Value { key } => doc.node(key).span.clone(),
        _ if matches!(n.kind, NodeKind::Mapping(_)) => doc
            .entries(node)
            .next()
            .map_or_else(|| n.span.clone(), |(key, _)| doc.node(key).span.clone()),
        _ => n.span.clone(),
    }
}

#[cfg(test)]
mod tests;
