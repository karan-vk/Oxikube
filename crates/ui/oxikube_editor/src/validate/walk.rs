//! The walk: a document tree and a schema, visited together.

use std::collections::HashMap;
use std::ops::Range;

use oxikube_domain::schema::{JsonSchema, SchemaType};
use regex::{Regex, RegexBuilder};

use super::diagnostic::{Diagnostic, DiagnosticCode, Severity};
use super::options::ValidateOptions;
use super::scalar::{ValueType, matches_enum, scalar_type};
use super::suggest::nearest;
use crate::yaml::{DocTree, NodeId, NodeKind, ScalarStyle};

/// Compiled `pattern`s, by pattern text. A pattern that does not compile (a Go-only construct)
/// is remembered as such and never reported: the validator must not invent errors.
#[derive(Default)]
pub(super) struct PatternCache(HashMap<String, Option<Regex>>);

impl PatternCache {
    fn is_match(&mut self, pattern: &str, text: &str) -> bool {
        if !self.0.contains_key(pattern) {
            let compiled = RegexBuilder::new(pattern).size_limit(1 << 20).build().ok();
            self.0.insert(pattern.to_owned(), compiled);
        }
        self.0
            .get(pattern)
            .and_then(Option::as_ref)
            .is_none_or(|re| re.is_match(text))
    }
}

/// The state of validating one document.
pub(super) struct Walk<'a> {
    pub(super) text: &'a str,
    pub(super) doc: &'a DocTree,
    pub(super) opts: &'a ValidateOptions,
    pub(super) patterns: &'a mut PatternCache,
    /// Start offsets of the document's syntax errors, sorted. Text near one is missing from the
    /// partial tree, so `required` is not judged there.
    pub(super) errors: &'a [usize],
    pub(super) out: Vec<Diagnostic>,
}

const INT_OR_STRING: [SchemaType; 2] = [SchemaType::Integer, SchemaType::String];

/// The types a node may have; empty means anything.
fn allowed_types(schema: &JsonSchema) -> &[SchemaType] {
    if schema.types.is_empty() && schema.xk8s.int_or_string {
        &INT_OR_STRING
    } else {
        &schema.types
    }
}

impl Walk<'_> {
    fn full(&self) -> bool {
        self.out.len() >= self.opts.max_diagnostics
    }

    pub(super) fn report(
        &mut self,
        node: NodeId,
        span: Range<usize>,
        severity: Severity,
        code: DiagnosticCode,
        message: String,
    ) {
        if self.full() {
            return;
        }
        self.out.push(Diagnostic {
            doc: self.doc.index,
            span,
            severity,
            message,
            code,
            path: self.doc.path_of(node, self.text),
        });
    }

    /// [`report`](Self::report) underlining the whole of `node`.
    pub(super) fn report_node(
        &mut self,
        node: NodeId,
        severity: Severity,
        code: DiagnosticCode,
        message: String,
    ) {
        let span = self.doc.node(node).span.clone();
        self.report(node, span, severity, code, message);
    }

    /// Whether a syntax error sits in or right after `span`: the mapping there may have lost
    /// keys to the recovery, so a missing one proves nothing.
    pub(super) fn near_error(&self, span: &Range<usize>) -> bool {
        self.errors.iter().any(|&at| {
            span.start <= at
                && (at <= span.end
                    || self.text.get(span.end..at).is_some_and(|gap| {
                        gap.split_once('\n')
                            .is_none_or(|(_, rest)| rest.trim().is_empty())
                    }))
        })
    }

    /// Checks the node `id` against `schema`. `is_root` is the document's root mapping.
    pub(super) fn walk(&mut self, id: NodeId, schema: &JsonSchema, is_root: bool) {
        if schema.truncated || self.full() {
            return;
        }
        match self.doc.node(id).kind {
            // Aliases and merge keys are not expanded: nothing to judge here.
            NodeKind::Alias { .. } => {}
            NodeKind::Scalar(style) => self.scalar(id, style, schema),
            NodeKind::Mapping(_) => self.mapping(id, schema, is_root),
            NodeKind::Sequence(_) => self.sequence(id, schema),
        }
    }

    /// `false` (after reporting `type-mismatch`) when the value's type is not one the schema allows.
    pub(super) fn check_type(
        &mut self,
        id: NodeId,
        schema: &JsonSchema,
        ty: ValueType,
        text: &str,
    ) -> bool {
        let types = allowed_types(schema);
        if types.is_empty() || types.iter().any(|t| ty.satisfies(*t, text)) {
            return true;
        }
        let expected = types
            .iter()
            .map(|t| t.as_str())
            .collect::<Vec<_>>()
            .join(" or ");
        self.report_node(
            id,
            Severity::Error,
            DiagnosticCode::TypeMismatch,
            format!("expected {expected}, found {}", describe(ty, text)),
        );
        false
    }

    fn scalar(&mut self, id: NodeId, style: ScalarStyle, schema: &JsonSchema) {
        let text = self.doc.scalar_value(id, self.text);
        let ty = match explicit_tag(self.text, self.doc.node(id).span.start) {
            None => scalar_type(style, text),
            Some(tag) => match tag_type(tag) {
                Some(ty) => ty,
                // A tag the validator does not know (`!custom`): the value is not what it looks like.
                None => return,
            },
        };
        // The server reads a null as "not set": an empty value is not a type error.
        if ty == ValueType::Null || !self.check_type(id, schema, ty, text) {
            return;
        }
        if !schema.enum_values.is_empty()
            && !schema
                .enum_values
                .iter()
                .any(|allowed| matches_enum(ty, text, allowed))
        {
            let message = enum_message(text, ty, schema);
            self.report_node(id, Severity::Error, DiagnosticCode::Enum, message);
        }
        if ty == ValueType::String
            && let Some(pattern) = &schema.pattern
            && !self.patterns.is_match(pattern, text)
        {
            self.report_node(
                id,
                Severity::Error,
                DiagnosticCode::Pattern,
                format!("{} does not match the pattern {pattern}", quote(text)),
            );
        }
    }

    /// The `unknown-field` warning for the key `key` of a mapping with `schema`.
    pub(super) fn unknown_field(&mut self, key: NodeId, name: &str, schema: &JsonSchema) {
        let mut message = format!("unknown field {}", quote(name));
        if let Some(near) = nearest(name, schema.properties.keys()) {
            message.push_str(&format!(" (did you mean {}?)", quote(near)));
        }
        self.report_node(
            key,
            Severity::Warning,
            DiagnosticCode::UnknownField,
            message,
        );
    }
}

/// The entry of mapping `map` whose key reads `name`: the key node and its value, if any.
pub(super) fn find_entry(
    doc: &DocTree,
    text: &str,
    map: NodeId,
    name: &str,
) -> Option<(NodeId, Option<NodeId>)> {
    doc.entries(map)
        .find(|(key, _)| doc.scalar_value(*key, text) == name)
}

/// How a value is named in a `type-mismatch` message: `string "x"`, `integer 3`, `object`.
pub(super) fn describe(ty: ValueType, text: &str) -> String {
    match ty {
        ValueType::String => format!("string {}", quote(text)),
        ValueType::Int | ValueType::Float | ValueType::Bool => format!("{} {text}", ty.name()),
        ValueType::Null | ValueType::Object | ValueType::Array => ty.name().to_owned(),
    }
}

/// Most bytes `explicit_tag` looks back over: a tag or anchor is never this long, and the bound
/// keeps validation linear on a whitespace-free (minified JSON) buffer.
const TAG_LOOKBACK: usize = 256;

/// Whether `byte` ends the token before it: whitespace, or a flow-collection indicator.
fn ends_token(byte: u8) -> bool {
    matches!(
        byte,
        b' ' | b'\t' | b'\n' | b'\r' | b',' | b'[' | b']' | b'{' | b'}'
    )
}

/// The explicit tag (`!!str`, `!custom`) written before the scalar starting at `start`, on its
/// line, past any anchors. The model's spans exclude tags and anchors. Only the
/// [`TAG_LOOKBACK`] bytes before `start` are read, so the cost per scalar is constant.
fn explicit_tag(text: &str, start: usize) -> Option<&str> {
    let bytes = text.as_bytes().get(..start)?;
    let floor = start.saturating_sub(TAG_LOOKBACK);
    let mut end = start;
    loop {
        while end > floor && matches!(bytes[end - 1], b' ' | b'\t') {
            end -= 1;
        }
        let token_start = bytes[floor..end]
            .iter()
            .rposition(|b| ends_token(*b))
            .map_or(floor, |at| floor + at + 1);
        let token = text.get(token_start..end)?;
        if token.starts_with('&') && token_start > floor {
            end = token_start;
        } else {
            return token.starts_with('!').then_some(token);
        }
    }
}

/// The type an explicit YAML core tag gives its scalar; `None` for any other tag.
fn tag_type(tag: &str) -> Option<ValueType> {
    Some(match tag {
        "!!str" | "!!binary" | "!!timestamp" => ValueType::String,
        "!!int" => ValueType::Int,
        "!!float" => ValueType::Float,
        "!!bool" => ValueType::Bool,
        "!!null" => ValueType::Null,
        _ => return None,
    })
}

fn enum_message(text: &str, ty: ValueType, schema: &JsonSchema) -> String {
    const SHOWN: usize = 10;
    let allowed: Vec<String> = schema
        .enum_values
        .iter()
        .take(SHOWN)
        .map(serde_json::Value::to_string)
        .collect();
    let more = if schema.enum_values.len() > SHOWN {
        ", ..."
    } else {
        ""
    };
    let shown = if ty == ValueType::String {
        quote(text)
    } else {
        text.to_owned()
    };
    let mut message = format!(
        "invalid value {shown}, expected one of {}{more}",
        allowed.join(", ")
    );
    if ty == ValueType::String
        && let Some(near) = nearest(
            text,
            schema
                .enum_values
                .iter()
                .filter_map(serde_json::Value::as_str),
        )
    {
        message.push_str(&format!(" (did you mean {}?)", quote(near)));
    }
    message
}

/// `"text"`, shortened to 40 characters so a long value does not fill the tooltip.
pub(super) fn quote(text: &str) -> String {
    const MAX: usize = 40;
    match text.char_indices().nth(MAX) {
        Some((cut, _)) => format!("{:?}...", &text[..cut]),
        None => format!("{text:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag_before(text: &str, value: &str) -> Option<String> {
        let start = text.rfind(value).expect("value in text");
        explicit_tag(text, start).map(str::to_owned)
    }

    #[test]
    fn finds_tags_past_anchors() {
        assert_eq!(tag_before("s: !!str 3", "3").as_deref(), Some("!!str"));
        assert_eq!(tag_before("s: &a !!str 3", "3").as_deref(), Some("!!str"));
        assert_eq!(tag_before("s: !!int &b 3", "3").as_deref(), Some("!!int"));
        assert_eq!(tag_before("s: 3", "3"), None);
        assert_eq!(tag_before("[!!str 3]", "3").as_deref(), Some("!!str"));
        assert_eq!(
            tag_before("{a: !custom 3}", "3").as_deref(),
            Some("!custom")
        );
        assert_eq!(tag_before("3", "3"), None);
    }

    #[test]
    fn does_not_look_past_the_lookback_or_the_line() {
        // A tag on an earlier line is not this scalar's tag.
        assert_eq!(tag_before("a: !!str\n3", "3"), None);
        // A whitespace-free prefix much longer than the bound reads only the bound.
        let long = format!("{{\"{}\":3}}", "k".repeat(10 * TAG_LOOKBACK));
        assert_eq!(tag_before(&long, "3"), None);
        let tagged = format!("!{} 3", "t".repeat(10 * TAG_LOOKBACK));
        assert_eq!(tag_before(&tagged, "3"), None);
    }

    #[test]
    fn lookback_lands_on_char_boundaries() {
        let text = format!("{}é 3", "é".repeat(TAG_LOOKBACK));
        assert_eq!(tag_before(&text, "3"), None);
        let text = format!("{}3", "é".repeat(TAG_LOOKBACK));
        assert_eq!(tag_before(&text, "3"), None);
    }
}
