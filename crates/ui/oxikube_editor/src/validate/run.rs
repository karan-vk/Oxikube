//! The entry points: one document against one schema, a set of [`Unit`]s, and a whole buffer.

use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::JsonSchema;

use super::diagnostic::{Diagnostic, DiagnosticCode, Severity};
use super::options::ValidateOptions;
use super::units::{Unit, node_gvk, units};
use super::walk::{PatternCache, Walk};
use crate::cancel::{CancelToken, Cancelled};
use crate::yaml::{DocTree, JsonPath, NodeId, ParseResult};

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
    let Some(root) = doc.root() else {
        return Vec::new();
    };
    let mut patterns = PatternCache::default();
    let mut out =
        validate_node(parsed, doc, root, schema, opts, &mut patterns, &never()).unwrap_or_default();
    // The walk reports a mapping's missing keys after its children; the editor wants source order.
    out.sort_by_key(|d| (d.span.start, d.span.end));
    out
}

/// A token nobody cancels, for the entry points that always run to the end.
fn never() -> CancelToken {
    CancelToken::new()
}

/// Walks the object at `node` of `doc` against `schema` (as a root: its `status` is skipped by
/// default). Unsorted.
fn validate_node(
    parsed: &ParseResult,
    doc: &DocTree,
    node: NodeId,
    schema: &JsonSchema,
    opts: &ValidateOptions,
    patterns: &mut PatternCache,
    cancel: &CancelToken,
) -> Result<Vec<Diagnostic>, Cancelled> {
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
        cancel,
        steps: 0,
        cancelled: false,
    };
    walk.walk(node, schema, true);
    if walk.cancelled {
        return Err(Cancelled);
    }
    Ok(walk.out)
}

/// The buffer's syntax errors as diagnostics (code `syntax`, severity error), at most
/// `opts.max_diagnostics`, in buffer order.
#[must_use]
pub fn syntax_diagnostics(parsed: &ParseResult, opts: &ValidateOptions) -> Vec<Diagnostic> {
    parsed
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
        .collect()
}

/// Validates each of `units` (see [`units`](super::units)) against the schema `schema_for`
/// returns for its kind; a unit whose kind has no schema (the cluster does not serve it, or it has
/// not arrived yet) gets no diagnostics. Stops after `opts.max_diagnostics` findings.
///
/// Cooperative: `cancel` is polled before each unit and every few hundred nodes inside one, and
/// a cancelled run returns [`Cancelled`] at the next poll with its findings dropped. `schema_for`
/// is called once per unit, in order, and does no I/O of its own. The result is sorted by
/// position.
pub fn validate_units(
    parsed: &ParseResult,
    units: &[Unit],
    mut schema_for: impl FnMut(&Gvk) -> Option<Arc<JsonSchema>>,
    opts: &ValidateOptions,
    cancel: &CancelToken,
) -> Result<Vec<Diagnostic>, Cancelled> {
    let mut out: Vec<Diagnostic> = Vec::new();
    let mut patterns = PatternCache::default();
    for unit in units {
        cancel.check()?;
        if out.len() >= opts.max_diagnostics {
            break;
        }
        let Some(schema) = schema_for(&unit.gvk) else {
            continue;
        };
        let Some(doc) = parsed.docs().get(unit.doc) else {
            continue;
        };
        let limited = ValidateOptions {
            max_diagnostics: opts.max_diagnostics - out.len(),
            ..opts.clone()
        };
        out.extend(validate_node(
            parsed,
            doc,
            unit.node,
            &schema,
            &limited,
            &mut patterns,
            cancel,
        )?);
    }
    out.sort_by_key(|d| (d.span.start, d.span.end));
    Ok(out)
}

/// Validates every object of `parsed` (each document, or each item of a list document; see
/// [`units`](super::units)) against the schema `schema_for` returns for its `apiVersion` and
/// `kind`, and reports the buffer's syntax errors (code `syntax`, severity error). An object
/// without those two fields, or whose kind has no schema, gets no schema diagnostics.
///
/// `schema_for` is called once per object, in order, and does no I/O of its own: the caller
/// fetches schemas (`SchemaPort`) first and answers from what it holds. The result is sorted by
/// position.
#[must_use]
pub fn validate_buffer(
    parsed: &ParseResult,
    schema_for: impl FnMut(&Gvk) -> Option<Arc<JsonSchema>>,
    opts: &ValidateOptions,
) -> Vec<Diagnostic> {
    let mut out = syntax_diagnostics(parsed, opts);
    let limited = ValidateOptions {
        max_diagnostics: opts.max_diagnostics.saturating_sub(out.len()),
        ..opts.clone()
    };
    if limited.max_diagnostics > 0 {
        let all = units(parsed);
        out.extend(
            validate_units(parsed, &all, schema_for, &limited, &never()).unwrap_or_default(),
        );
    }
    out.sort_by_key(|d| (d.span.start, d.span.end));
    out
}

/// The kind a document declares: its root `apiVersion` and `kind`, when both are non-empty
/// strings.
#[must_use]
pub fn document_gvk(parsed: &ParseResult, doc: &DocTree) -> Option<Gvk> {
    node_gvk(parsed, doc, doc.root()?)
}
