//! The entry points: one document against one schema, and a whole buffer.

use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::JsonSchema;

use super::diagnostic::{Diagnostic, DiagnosticCode, Severity};
use super::options::ValidateOptions;
use super::scalar::{ValueType, scalar_type};
use super::walk::{PatternCache, Walk, find_entry};
use crate::yaml::{DocTree, JsonPath, NodeKind, ParseResult};

/// Validates one document of `parsed` against `schema`: unknown fields, wrong types, enums,
/// required fields, patterns, int-or-string values and the `x-kubernetes-*` hints. Pure and
/// synchronous, so callers run it on the background executor and may drop the result when the
/// buffer has moved on.
///
/// `doc` must be one of `parsed.docs()`. Spans are byte ranges of the whole buffer. A document
/// with syntax errors is validated as far as its partial tree goes; `required` is not judged on
/// objects a syntax error touched, since the recovery may have dropped their keys. The syntax
/// errors themselves are not repeated here; [`validate_buffer`] adds them. Diagnostics are sorted by
/// position.
#[must_use]
pub fn validate(
    parsed: &ParseResult,
    doc: &DocTree,
    schema: &JsonSchema,
    opts: &ValidateOptions,
) -> Vec<Diagnostic> {
    validate_doc(parsed, doc, schema, opts, &mut PatternCache::default())
}

fn validate_doc(
    parsed: &ParseResult,
    doc: &DocTree,
    schema: &JsonSchema,
    opts: &ValidateOptions,
    patterns: &mut PatternCache,
) -> Vec<Diagnostic> {
    let Some(root) = doc.root() else {
        return Vec::new();
    };
    let mut errors: Vec<usize> = parsed
        .diagnostics()
        .iter()
        .filter(|d| d.doc == doc.index)
        .map(|d| d.span.start)
        .collect();
    errors.sort_unstable();
    let mut walk = Walk {
        text: parsed.text(),
        doc,
        opts,
        patterns,
        errors: &errors,
        out: Vec::new(),
    };
    walk.walk(root, schema, true);
    // The walk reports a mapping's missing keys after its children; the editor wants source order.
    let mut out = walk.out;
    out.sort_by_key(|d| (d.span.start, d.span.end));
    out
}

/// Validates every document of `parsed` against the schema `schema_for` returns for its
/// `apiVersion` and `kind`, and reports the buffer's syntax errors (code `syntax`, severity
/// error). A document without those two fields, or whose kind has no schema (the cluster does
/// not serve it, or the schema has not arrived yet), gets no schema diagnostics.
///
/// `schema_for` is called once per document, in order, and does no I/O of its own: the caller
/// fetches schemas (`SchemaPort`) first and answers from what it holds. The result is sorted by
/// position.
#[must_use]
pub fn validate_buffer(
    parsed: &ParseResult,
    mut schema_for: impl FnMut(&Gvk) -> Option<Arc<JsonSchema>>,
    opts: &ValidateOptions,
) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = parsed
        .diagnostics()
        .iter()
        .take(opts.max_diagnostics)
        .map(|d| Diagnostic {
            doc: d.doc,
            span: d.span.clone(),
            severity: Severity::Error,
            message: d.message.clone(),
            code: DiagnosticCode::Syntax,
            path: JsonPath::root(),
        })
        .collect();
    let mut patterns = PatternCache::default();
    for doc in parsed.docs() {
        if out.len() >= opts.max_diagnostics {
            break;
        }
        let Some(gvk) = document_gvk(parsed, doc) else {
            continue;
        };
        let Some(schema) = schema_for(&gvk) else {
            continue;
        };
        let room = opts.max_diagnostics - out.len();
        let limited = ValidateOptions {
            max_diagnostics: room,
            ..opts.clone()
        };
        out.extend(validate_doc(parsed, doc, &schema, &limited, &mut patterns));
    }
    out.sort_by_key(|d| (d.span.start, d.span.end));
    out
}

/// The kind a document declares: its root `apiVersion` and `kind`, when both are non-empty
/// strings.
#[must_use]
pub fn document_gvk(parsed: &ParseResult, doc: &DocTree) -> Option<Gvk> {
    let root = doc.root()?;
    if !matches!(doc.node(root).kind, NodeKind::Mapping(_)) {
        return None;
    }
    let field = |name: &str| {
        let value = find_entry(doc, parsed.text(), root, name)?.1?;
        let NodeKind::Scalar(style) = doc.node(value).kind else {
            return None;
        };
        let text = doc.scalar_value(value, parsed.text());
        (!text.is_empty() && scalar_type(style, text) == ValueType::String).then_some(text)
    };
    Some(Gvk::from_api_version(field("apiVersion")?, field("kind")?))
}
