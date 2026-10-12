//! Drawing an [`AllocationView`]: one block per resource, a bar and the numbers under it.
//!
//! Colour never carries a number alone: every segment has its line of text (`Requests`, `Limits`,
//! `Usage`), and the swatch beside it only ties the line to the segment. Everything drawn is
//! ready-made in the view; a frame builds elements and computes nothing from the cluster.

use gpui::{
    AnyElement, Div, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    div, px,
};
use oxikube_app::allocation::{AllocationKind, AllocationRow, AllocationView, BarModel, Usage};
use oxikube_ui::allocation_bar::AllocationBar;
use oxikube_ui::chart::{ChartPalette, Tone};
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{Tokens, u};

use crate::table::ToneColors;

/// Requests past this share of the capacity turn the bar amber, past [`FULL`] red.
const BUSY: f32 = 0.7;
/// See [`BUSY`].
const FULL: f32 = 0.9;

/// The width of the title column of a row (`Ephemeral storage`, `GPU nvidia.com/gpu`), before the UI
/// zoom; longer titles are cut with an ellipsis.
const TITLE_WIDTH: f32 = 112.;
/// The width of the capacity column (`of 3920m allocatable`), before the UI zoom: bars of every row
/// start and end in the same place.
const CAPACITY_WIDTH: f32 = 138.;

/// The colour a bar's fill takes: green, amber from [`BUSY`], red from [`FULL`] or when the
/// amount reaches the capacity.
pub(super) fn tone_of(bar: &BarModel) -> Tone {
    if bar.full || bar.fill >= FULL {
        Tone::Critical
    } else if bar.fill >= BUSY {
        Tone::Warn
    } else {
        Tone::Good
    }
}

fn muted(tokens: &Tokens, text: impl Into<gpui::SharedString>) -> Div {
    div()
        .text_size(u(tokens.font.small))
        .text_color(tokens.colors.text_muted)
        .child(text.into())
}

/// The section for `view`: a block per row. `id` prefixes the element ids, so a row is
/// `{id}-cpu`, its bar `{id}-cpu-bar` / `{id}-cpu-fill` / `{id}-cpu-usage` / `{id}-cpu-limits`
/// (the segments) and its lines `{id}-cpu-requests`, `-limits-text`, `-usage-text`, `-unset`,
/// `-overcommit`.
pub fn allocation_section(
    tokens: &Tokens,
    tones: &ToneColors,
    view: &AllocationView,
    id: &'static str,
) -> AnyElement {
    v_flex()
        .debug_selector(move || id.to_owned())
        .gap(u(tokens.spacing.md))
        .children(
            view.rows
                .iter()
                .map(|row| row_block(tokens, tones, row, view.capacity_name(), id)),
        )
        .children(view.notes.iter().enumerate().map(|(ix, note)| {
            muted(tokens, note.clone()).debug_selector(move || format!("{id}-note-{ix}"))
        }))
        .into_any_element()
}

fn row_block(
    tokens: &Tokens,
    tones: &ToneColors,
    row: &AllocationRow,
    capacity_name: &str,
    id: &str,
) -> AnyElement {
    let row_id = format!("{id}-{}", row.kind.id());
    let bar = row.bar();
    let tone = bar.as_ref().map_or(Tone::Good, tone_of);
    let capacity = row.capacity_text().map_or_else(
        || "no cap".to_owned(),
        |c| format!("of {c} {capacity_name}"),
    );
    let selector = row_id.clone();
    v_flex()
        .debug_selector(move || selector)
        .gap(u(px(2.)))
        .child(
            h_flex()
                .items_center()
                .gap(u(tokens.spacing.md))
                .child(
                    div()
                        .w(u(px(TITLE_WIDTH)))
                        .flex_none()
                        .truncate()
                        .font_semibold()
                        .child(row.kind.title()),
                )
                .child(div().flex_1().children(bar.map(|bar| {
                    AllocationBar::new(bar.fill)
                        .tone(tone_of(&bar))
                        .limits_mark(bar.limits_mark)
                        .usage(bar.usage)
                        .selector(row_id.clone())
                })))
                .child(
                    muted(tokens, capacity)
                        .w(u(px(CAPACITY_WIDTH)))
                        .flex_none()
                        .truncate()
                        .text_right(),
                ),
        )
        .child(segments(tokens, row, tone, &row_id))
        .children(notes(tokens, tones, row, &row_id))
        .into_any_element()
}

/// The numbers of a row, one chip per segment: `[swatch] Requests 980m (25%)`.
fn segments(tokens: &Tokens, row: &AllocationRow, tone: Tone, id: &str) -> AnyElement {
    let palette = ChartPalette::from_tokens(tokens);
    let requests_label = match row.kind {
        AllocationKind::Pods => "Pods",
        _ => "Requests",
    };
    let mut out = h_flex()
        .flex_wrap()
        .gap_x(u(tokens.spacing.lg))
        .gap_y(u(px(1.)));
    out = out.child(segment(
        tokens,
        Swatch::Fill(palette.tone(tone)),
        requests_label,
        row.requests_short(),
        format!("{id}-requests"),
    ));
    if let Some(text) = row.limits_short() {
        out = out.child(segment(
            tokens,
            Swatch::Tick(tokens.colors.text),
            "Limits",
            text,
            format!("{id}-limits-text"),
        ));
    }
    if let Some(text) = row.usage_short() {
        out = out.child(segment(
            tokens,
            Swatch::Fill(palette.info),
            "Usage",
            text,
            format!("{id}-usage-text"),
        ));
    }
    out.into_any_element()
}

/// The lines under the numbers: why there is no usage, over-commitment, pods that set nothing.
fn notes(tokens: &Tokens, tones: &ToneColors, row: &AllocationRow, id: &str) -> Vec<AnyElement> {
    let mut out = Vec::new();
    let mut note = |text: String, color: Hsla, selector: String| {
        out.push(
            div()
                .debug_selector(move || selector)
                .text_size(u(tokens.font.small))
                .text_color(color)
                .child(text)
                .into_any_element(),
        );
    };
    match &row.usage {
        // An absence reads in the warning colour, so it is not mistaken for a quiet resource.
        Usage::Unavailable(reason) => {
            note(reason.to_string(), tones.warn, format!("{id}-usage-text"));
        }
        Usage::Pending => note(
            "reading usage...".to_owned(),
            tokens.colors.text_muted,
            format!("{id}-usage-text"),
        ),
        Usage::NotReported => note(
            row.usage_text().unwrap_or_default(),
            tokens.colors.text_muted,
            format!("{id}-usage-note"),
        ),
        Usage::Value(_) | Usage::NotApplicable => {}
    }
    if let Some(text) = row.overcommit_text() {
        note(text, tokens.colors.text_muted, format!("{id}-overcommit"));
    }
    if let Some(text) = row.unset_text() {
        note(text, tokens.colors.text_muted, format!("{id}-unset"));
    }
    out
}

/// How a segment's swatch looks: a square for the bar's fills, a thin tick for the limits.
enum Swatch {
    Fill(Hsla),
    Tick(Hsla),
}

/// `[swatch] Label text`.
fn segment(
    tokens: &Tokens,
    swatch: Swatch,
    label: &'static str,
    text: String,
    id: String,
) -> AnyElement {
    let (width, color) = match swatch {
        Swatch::Fill(color) => (8., color),
        Swatch::Tick(color) => (2., color),
    };
    h_flex()
        .debug_selector(move || id)
        .gap(u(tokens.spacing.sm))
        .items_center()
        .text_size(u(tokens.font.small))
        .child(
            div()
                .w(u(px(width)))
                .h(u(px(8.)))
                .flex_none()
                .rounded(u(px(2.)))
                .bg(color),
        )
        .child(div().text_color(tokens.colors.text_muted).child(label))
        .child(div().text_color(tokens.colors.text).child(text))
        .into_any_element()
}
