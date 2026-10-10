//! Byte offsets to the library's positions, in one place.
//!
//! The spanned YAML model and the validator speak UTF-8 byte offsets into the whole buffer;
//! gpui-component's `DiagnosticSet` takes line/character [`Position`]s. Every conversion goes
//! through here, so a span that is past the end or inside a multi-byte character is clamped the
//! same way everywhere, and the per-line summary the overlay draws agrees with the squiggles.

use std::ops::Range;

use gpui_component::highlighter::{Diagnostic, DiagnosticSeverity};
use gpui_component::input::{Position, Rope, RopeExt as _};

use super::api::{DiagnosticLevel, EditorDiagnostic};

/// `range` clamped to the buffer, as positions; an empty range grows to the character after it
/// (or the one before it at the end of a line) so the squiggle is visible.
pub(crate) fn span_to_positions(rope: &Rope, range: &Range<usize>) -> Range<Position> {
    let len = rope.len();
    let start = rope.offset_to_position(range.start.min(len));
    let end = rope.offset_to_position(range.end.clamp(range.start.min(len), len));
    if start != end {
        return start..end;
    }
    let line_chars = line_chars(rope, start.line);
    if start.character < line_chars {
        start..Position::new(start.line, start.character + 1)
    } else if start.character > 0 {
        Position::new(start.line, start.character - 1)..start
    } else {
        start..end
    }
}

/// The number of characters on `line`, without its line break.
fn line_chars(rope: &Rope, line: u32) -> u32 {
    rope.offset_to_position(rope.line_end_offset(line as usize))
        .character
}

/// The library's diagnostic for one of ours.
pub(crate) fn to_library(rope: &Rope, diagnostic: &EditorDiagnostic) -> Diagnostic {
    let severity = match diagnostic.level {
        DiagnosticLevel::Error => DiagnosticSeverity::Error,
        DiagnosticLevel::Warning => DiagnosticSeverity::Warning,
        DiagnosticLevel::Info => DiagnosticSeverity::Info,
        DiagnosticLevel::Hint => DiagnosticSeverity::Hint,
    };
    let mut out = Diagnostic::new(
        span_to_positions(rope, &diagnostic.range),
        diagnostic.message.clone(),
    )
    .with_severity(severity)
    .with_source("oxikube");
    if let Some(code) = &diagnostic.code {
        out = out.with_code(code.clone());
    }
    out
}

/// What the overlay draws for one line: the worst level among the line's diagnostics, the first
/// message and how many there are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LineSummary {
    /// Zero-based buffer line.
    pub(crate) line: usize,
    /// Byte offset of the start of the line (where the marker is drawn).
    pub(crate) line_start: usize,
    /// Byte offset of the end of the line (where the message is drawn).
    pub(crate) line_end: usize,
    /// The most severe level on the line.
    pub(crate) level: DiagnosticLevel,
    /// The first diagnostic's message, on one line.
    pub(crate) message: String,
    /// How many diagnostics start on the line.
    pub(crate) count: usize,
}

/// One summary per line that has a diagnostic, in line order. `diagnostics` must be sorted by
/// start offset.
pub(crate) fn line_summaries(rope: &Rope, diagnostics: &[EditorDiagnostic]) -> Vec<LineSummary> {
    let mut out: Vec<LineSummary> = Vec::new();
    for diagnostic in diagnostics {
        let line = span_to_positions(rope, &diagnostic.range).start.line as usize;
        match out.last_mut() {
            Some(last) if last.line == line => {
                last.count += 1;
                last.level = last.level.min(diagnostic.level);
            }
            _ => out.push(LineSummary {
                line,
                line_start: rope.line_start_offset(line),
                line_end: rope.line_end_offset(line),
                level: diagnostic.level,
                message: first_line(&diagnostic.message),
                count: 1,
            }),
        }
    }
    out
}

fn first_line(message: &str) -> String {
    message.lines().next().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diag(range: Range<usize>, level: DiagnosticLevel) -> EditorDiagnostic {
        EditorDiagnostic::new(range, level, "bad\nmore")
    }

    #[test]
    fn spans_become_positions_and_are_clamped() {
        let rope = Rope::from("kind: Pod\nmetadata:\n  näme: x\n");
        assert_eq!(
            span_to_positions(&rope, &(0..4)),
            Position::new(0, 0)..Position::new(0, 4)
        );
        // `näme` starts at byte 22; the `ä` is two bytes, characters count once.
        assert_eq!(
            span_to_positions(&rope, &(22..27)),
            Position::new(2, 2)..Position::new(2, 6)
        );
        // Past the end clamps to the end.
        let end = rope.offset_to_position(rope.len());
        assert_eq!(span_to_positions(&rope, &(500..900)).end, end);
    }

    #[test]
    fn an_empty_span_underlines_a_character() {
        let rope = Rope::from("a: 1\nb:\n");
        assert_eq!(
            span_to_positions(&rope, &(0..0)),
            Position::new(0, 0)..Position::new(0, 1)
        );
        // At the end of `b:` the character before it.
        assert_eq!(
            span_to_positions(&rope, &(7..7)),
            Position::new(1, 1)..Position::new(1, 2)
        );
    }

    #[test]
    fn library_diagnostics_keep_level_and_code() {
        let rope = Rope::from("a: 1\n");
        let d = to_library(
            &rope,
            &diag(0..1, DiagnosticLevel::Warning).with_code("unknown-field"),
        );
        assert_eq!(d.severity, DiagnosticSeverity::Warning);
        assert_eq!(d.code.as_deref(), Some("unknown-field"));
        assert_eq!(d.range, Position::new(0, 0)..Position::new(0, 1));
    }

    #[test]
    fn summaries_group_by_line_and_keep_the_worst_level() {
        let rope = Rope::from("a: 1\nb: 2\nc: 3\n");
        let summaries = line_summaries(
            &rope,
            &[
                diag(0..1, DiagnosticLevel::Warning),
                diag(3..4, DiagnosticLevel::Error),
                diag(10..11, DiagnosticLevel::Info),
            ],
        );
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].line, 0);
        assert_eq!(summaries[0].count, 2);
        assert_eq!(summaries[0].level, DiagnosticLevel::Error);
        assert_eq!(summaries[0].message, "bad");
        assert_eq!(summaries[0].line_end, 4);
        assert_eq!(summaries[1].line, 2);
    }
}
