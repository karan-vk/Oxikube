//! `parse`: buffer → [`ParseResult`] over granit-parser, the only module that knows its types.

use std::ops::Range;
use std::sync::Arc;

use granit_parser::{Event, Parser, ScalarStyle as GScalar, Span, StructureStyle, options};

use super::builder::{Builder, Shape};
use super::dupes::duplicate_keys;
use super::node::{CollectionStyle, NodeKind, ScalarStyle};
use super::recover::{Resume, SegmentEnds, find_resume, scan_start};
use super::result::{ParseResult, SyntaxDiagnostic};

/// Diagnostics per buffer; recovery gives up on the rest of the text when it reaches this many.
const MAX_DIAGNOSTICS: usize = 1_000;

/// Parses a YAML buffer (one or more `---` documents) into its spanned model. Never fails and
/// never panics: invalid YAML yields the tree built so far plus [`SyntaxDiagnostic`]s.
///
/// Copies `text` once into the result; use [`parse_shared`] when the caller already holds an
/// `Arc<str>`.
#[must_use]
pub fn parse(text: &str) -> ParseResult {
    parse_shared(Arc::from(text))
}

/// [`parse`] without copying the text.
///
/// Error recovery: granit-parser stops at the first error, so the parse restarts on a later line
/// (see `recover.rs`): at the next line whose indentation is that of a collection still open at
/// the error (that collection's block, up to the next line of an enclosing collection, is parsed
/// as its continuation), or at the next `---` (new documents). Text between the error and the
/// restart line is not in the tree. An alias in a restarted run cannot see anchors defined before
/// the error, so it is reported as an error too.
#[must_use]
pub fn parse_shared(text: Arc<str>) -> ParseResult {
    let mut builder = Builder::new();
    let mut diagnostics = Vec::new();
    let mut span = 0..text.len();
    let mut continuing = false;
    let mut segments = SegmentEnds::default();
    let end = loop {
        let outcome = run(&text, span.clone(), continuing, &mut builder);
        builder.end_run();
        let open: Vec<_> = builder.open_blocks().collect();
        let (resume, last_end) = match outcome {
            Ok(last_end) if span.end >= text.len() => break last_end,
            // A continued collection ended at a line of an enclosing one, or at a `---`.
            Ok(last_end) => (
                find_resume(&text, span.end, span.start, &open, false),
                last_end,
            ),
            Err(failure) => {
                diagnostics.push(SyntaxDiagnostic {
                    doc: builder.doc_index(),
                    span: error_span(&text, failure.at),
                    message: failure.message,
                });
                if diagnostics.len() >= MAX_DIAGNOSTICS {
                    break failure.last_end;
                }
                let from = scan_start(&text, failure.last_end);
                let empty = builder.doc_is_empty();
                (
                    find_resume(&text, from, span.start, &open, empty),
                    failure.last_end,
                )
            }
        };
        match resume {
            Some(Resume::Attach { at, depth }) => {
                builder.attach(depth);
                span = at..segments.get(&text, at, &open, depth);
                continuing = true;
            }
            Some(Resume::Adopt { at }) => {
                builder.adopt();
                span = at..text.len();
                continuing = false;
            }
            Some(Resume::NewDoc { at }) => {
                builder.abandon_doc(last_end);
                span = at..text.len();
                continuing = false;
            }
            None => break last_end,
        }
    };
    let docs = builder.finish(end);
    for doc in &docs {
        duplicate_keys(doc, &text, &mut diagnostics, MAX_DIAGNOSTICS);
    }
    diagnostics.sort_by_key(|d| d.span.start);
    ParseResult {
        text,
        docs,
        diagnostics,
    }
}

/// A parser run that stopped on a syntax error.
struct RunFailure {
    /// Error position in the buffer.
    at: usize,
    message: String,
    /// End of the last event the run produced (at least the run's start).
    last_end: usize,
}

/// Feeds one parser run over `text[span]` into the builder and returns the end of its last
/// event. A `continuing` run (the rest of a collection open at an error) adds to the current
/// document, so its document start and end are ignored.
fn run(
    text: &str,
    span: Range<usize>,
    continuing: bool,
    builder: &mut Builder,
) -> Result<usize, RunFailure> {
    let base = span.start;
    let source = &text[span];
    let mut last_end = base;
    for next in Parser::new_from_str_with_options(source, options! { emit_comments: false }) {
        let (event, span) = match next {
            Ok(pair) => pair,
            Err(error) => {
                let at = base + error.marker().byte_offset().unwrap_or(0);
                return Err(RunFailure {
                    at: at.clamp(last_end, text.len()),
                    message: error.info(),
                    last_end,
                });
            }
        };
        let range = absolute(&span, base, text.len());
        last_end = last_end.max(range.end);
        match event {
            Event::DocumentStart(..) | Event::DocumentEnd if continuing => {}
            Event::DocumentStart(explicit, _) => builder.doc_start(range, explicit),
            Event::DocumentEnd => builder.doc_end(range),
            Event::MappingStart(style, anchor, _) => {
                builder.collection_start(Shape::Mapping, collection_style(style), range, anchor);
            }
            Event::SequenceStart(style, anchor, _) => {
                let style = collection_style(style);
                let range = match style {
                    CollectionStyle::Block => sequence_range(text, range, builder.awaits_value()),
                    CollectionStyle::Flow => range,
                };
                builder.collection_start(Shape::Sequence, style, range, anchor);
            }
            Event::MappingEnd | Event::SequenceEnd => builder.collection_end(range),
            Event::Alias(id) => {
                let target = builder.anchor(id);
                builder.leaf(NodeKind::Alias { target }, range, None, 0);
            }
            Event::Scalar(value, style, anchor, _) => {
                let style = scalar_style(style);
                let range = scalar_range(text, range, style);
                let source = text.get(range.clone()).unwrap_or("");
                let implicit_null = style == ScalarStyle::Plain && range.is_empty();
                let decoded = (!implicit_null && value != source).then(|| value.into());
                builder.leaf(NodeKind::Scalar(style), range, decoded, anchor);
            }
            _ => {}
        }
    }
    Ok(last_end)
}

/// A span in buffer offsets. granit-parser always has byte offsets for `&str` input; a missing
/// one maps to the run's start.
fn absolute(span: &Span, base: usize, len: usize) -> Range<usize> {
    let range = span.byte_range().unwrap_or(0..0);
    let start = (base + range.start).min(len);
    start..(base + range.end).clamp(start, len)
}

/// A block scalar's span without the trailing blank lines and indentation the parser includes.
fn scalar_range(text: &str, range: Range<usize>, style: ScalarStyle) -> Range<usize> {
    if !matches!(style, ScalarStyle::Literal | ScalarStyle::Folded) {
        return range;
    }
    let source = text.get(range.clone()).unwrap_or("");
    let kept = source.trim_end_matches([' ', '\t', '\r', '\n']).len();
    range.start..range.start + kept
}

/// A block sequence starts at its first `-`. granit-parser starts an indentless sequence
/// (`key:\n- item`, items at the key's column) at its first item instead, which may itself start
/// with `-` (`- -c`, `- - x`). A block sequence that is a mapping value begins its line, so it
/// moves back to a `-` indicator that is the line's first token; otherwise to the nearest `-`
/// indicator before its start on the same line, unless it already starts at one.
fn sequence_range(text: &str, range: Range<usize>, mapping_value: bool) -> Range<usize> {
    let bytes = text.as_bytes();
    let line_start = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
    let indicator = |i: usize| {
        bytes.get(i) == Some(&b'-')
            && (i == line_start || matches!(bytes[i - 1], b' ' | b'\t'))
            && matches!(bytes.get(i + 1), None | Some(b' ' | b'\t' | b'\r' | b'\n'))
    };
    let first_token = (line_start..range.start).find(|&i| !matches!(bytes[i], b' ' | b'\t'));
    let start = match first_token.filter(|&i| mapping_value && indicator(i)) {
        Some(start) => start,
        None if indicator(range.start) => return range,
        None => match (line_start..range.start).rev().find(|&i| indicator(i)) {
            Some(start) => start,
            None => return range,
        },
    };
    start..range.end.max(start)
}

/// From the error position to the end of its line, or one character when the error is at a
/// line end.
fn error_span(text: &str, at: usize) -> Range<usize> {
    let rest = text.get(at..).unwrap_or("");
    let line = rest.find('\n').map_or(rest, |i| &rest[..i]);
    let line = line.trim_end_matches('\r');
    let len = if line.is_empty() {
        rest.chars().next().map_or(0, char::len_utf8)
    } else {
        line.len()
    };
    at..at + len
}

fn scalar_style(style: GScalar) -> ScalarStyle {
    match style {
        GScalar::SingleQuoted => ScalarStyle::SingleQuoted,
        GScalar::DoubleQuoted => ScalarStyle::DoubleQuoted,
        GScalar::Literal => ScalarStyle::Literal,
        GScalar::Folded => ScalarStyle::Folded,
        GScalar::Plain => ScalarStyle::Plain,
    }
}

fn collection_style(style: StructureStyle) -> CollectionStyle {
    match style {
        StructureStyle::Flow => CollectionStyle::Flow,
        StructureStyle::Block => CollectionStyle::Block,
    }
}
