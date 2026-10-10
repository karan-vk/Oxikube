//! The diagnostics overlay: a gutter marker and an end-of-line message per line with a finding,
//! painted in a `canvas` over the editor (gpui-component's editor has no inline widgets).
//!
//! The canvas is a later sibling of the editor element, so it paints after the editor has laid
//! out the frame and reads that frame's geometry through `EditorState::range_to_bounds`. Only the
//! lines of the visible range are looked at (a binary search into the per-line summaries), so a
//! buffer with thousands of findings costs what the screen shows.

use std::rc::Rc;

use gpui::{
    App, Bounds, ContentMask, Entity, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    TextAlign, TextRun, Window, canvas, div, fill, font, point, px, size,
};

use super::EditorState;
use super::api::DiagnosticLevel;
use super::positions::LineSummary;
use crate::size::u;
use crate::tokens::{ActiveTokens as _, Colors};

/// Width of the gutter marker, before zoom.
const MARKER_WIDTH: f32 = 3.;
/// Gap between a line's last character and its message, before zoom.
const MESSAGE_GAP: f32 = 24.;
/// Longest message drawn, in characters; the hover shows the whole text.
const MESSAGE_MAX_CHARS: usize = 160;

/// The colour of `level` in the active theme.
pub(crate) fn level_color(level: DiagnosticLevel, colors: &Colors) -> Hsla {
    match level {
        DiagnosticLevel::Error => colors.error,
        DiagnosticLevel::Warning => colors.warning,
        DiagnosticLevel::Info => colors.info,
        DiagnosticLevel::Hint => colors.text_muted,
    }
}

/// The text drawn after a line: the worst diagnostic's message, and how many more the line has.
pub(crate) fn message_text(summary: &LineSummary) -> String {
    let mut text: String = summary.message.chars().take(MESSAGE_MAX_CHARS).collect();
    if summary.message.chars().count() > MESSAGE_MAX_CHARS {
        text.push('…');
    }
    if summary.count > 1 {
        text.push_str(&format!("  (+{} more)", summary.count - 1));
    }
    text
}

/// The overlay element for `state` with `summaries` (sorted by line), placed over its parent.
/// Screen readers get one status element named `label` ("2 errors, 1 warning"); the painted
/// markers have no element of their own.
pub(crate) fn diagnostics_overlay(
    state: Entity<EditorState>,
    summaries: Rc<Vec<LineSummary>>,
    label: SharedString,
) -> impl IntoElement {
    div()
        .id("editor-diagnostics")
        .role(Role::Status)
        .aria_label(label)
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(
            canvas(
                |_, _, _| {},
                move |_, (), window, cx| paint(&state, &summaries, window, cx),
            )
            .size_full(),
        )
}

/// What the overlay's status element says: the count of errors and warnings.
pub(crate) fn status_label(summaries: &[LineSummary], errors: usize, warnings: usize) -> String {
    if summaries.is_empty() {
        return "No problems".to_owned();
    }
    let plural = |n: usize, word: &str| {
        if n == 1 {
            format!("1 {word}")
        } else {
            format!("{n} {word}s")
        }
    };
    format!(
        "{}, {}",
        plural(errors, "error"),
        plural(warnings, "warning")
    )
}

fn paint(
    state: &Entity<EditorState>,
    summaries: &[LineSummary],
    window: &mut Window,
    cx: &mut App,
) {
    if summaries.is_empty() {
        return;
    }
    let editor = state.read(cx);
    let Some(visible) = editor.visible_row_range() else {
        return;
    };
    let Some(line_height) = editor.line_height() else {
        return;
    };
    let viewport = editor.input_bounds();
    let first = summaries.partition_point(|s| s.line < visible.start);
    let lines: Vec<(Bounds<Pixels>, Bounds<Pixels>, &LineSummary)> = summaries[first..]
        .iter()
        .take_while(|s| s.line <= visible.end)
        .filter_map(|s| {
            // A line hidden in a fold has no bounds: nothing to draw.
            let start = editor.range_to_bounds(&(s.line_start..s.line_start))?;
            let end = editor.range_to_bounds(&(s.line_end..s.line_end))?;
            Some((start, end, s))
        })
        .collect();
    if lines.is_empty() {
        return;
    }
    let colors = cx.colors();
    let font_size = u(cx.tokens().font.small);
    let family = cx.mono_font_family();
    window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
        for (row, end, summary) in lines {
            let color = level_color(summary.level, &colors);
            let marker = Bounds::new(
                point(viewport.origin.x, row.origin.y),
                size(u(px(MARKER_WIDTH)), line_height),
            );
            window.paint_quad(fill(marker, color));
            let text: SharedString = message_text(summary).into();
            let run = TextRun {
                len: text.len(),
                font: font(family.clone()),
                color: color.opacity(0.85),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = window
                .text_system()
                .shape_line(text, font_size, &[run], None);
            let origin = point(
                end.origin.x + u(px(MESSAGE_GAP)),
                end.origin.y + (line_height - font_size) / 2.,
            );
            // A failed glyph raster only loses this frame's message.
            let _ = shaped.paint(origin, font_size, TextAlign::Left, None, window, cx);
        }
    });
}
