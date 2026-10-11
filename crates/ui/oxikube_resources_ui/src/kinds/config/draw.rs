//! Drawing a [`Panel`]: sections of label / value rows, bars, links and small tables.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px, relative,
};
use oxikube_app::columns::Tone;
use oxikube_domain::view::QuotaLevel;
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Tokens, u};

use super::model::{Panel, Row, Section};
use super::panel::ConfigPanel;
use crate::table::ToneColors;

/// The tallest the panel grows before it scrolls inside the drawer, unscaled pixels: the generic
/// Overview below it keeps most of the height.
const MAX_HEIGHT: f32 = 320.;
/// The width the labels of a section share, unscaled pixels (the Overview uses 150).
const LABEL_WIDTH: f32 = 150.;
/// The width of a bar's label: quota resources run long (`requests.nvidia.com/gpu`).
const BAR_LABEL_WIDTH: f32 = 210.;

/// The whole panel: a scrolling column of sections.
pub(super) fn panel(
    panel: &Panel,
    tokens: &Tokens,
    tones: &ToneColors,
    cx: &mut Context<ConfigPanel>,
) -> AnyElement {
    let mut bars = 0;
    let mut links = 0;
    let sections: Vec<AnyElement> = panel
        .sections
        .iter()
        .map(|section| draw_section(section, tokens, tones, &mut bars, &mut links, cx))
        .collect();
    v_flex()
        .id("config-panel")
        .debug_selector(|| "config-panel".to_owned())
        .w_full()
        .max_h(u(px(MAX_HEIGHT)))
        .overflow_y_scroll()
        .pb(u(tokens.spacing.sm))
        .border_b_1()
        .border_color(tokens.colors.border_variant)
        .children(sections)
        .into_any_element()
}

fn draw_section(
    section: &Section,
    tokens: &Tokens,
    tones: &ToneColors,
    bars: &mut usize,
    links: &mut usize,
    cx: &mut Context<ConfigPanel>,
) -> AnyElement {
    let mono = cx.mono_font_family();
    let rows = section.rows.iter().map(|row| match row {
        Row::Pair { label, value, tone } => pair(tokens, tones, label, value, *tone),
        Row::Bar {
            label,
            text,
            percent,
            level,
        } => {
            let ix = *bars;
            *bars += 1;
            bar(tokens, tones, ix, label, text, *percent, *level)
        }
        Row::Link { label, text, to } => {
            let ix = *links;
            *links += 1;
            let to = to.clone();
            line(tokens)
                .child(label_cell(tokens, label, LABEL_WIDTH))
                .child(
                    div()
                        .id(("config-link", ix))
                        .debug_selector(move || format!("config-link-{ix}"))
                        .flex_1()
                        .min_w_0()
                        .cursor_pointer()
                        .text_color(tokens.colors.accent)
                        .hover(|style| style.underline())
                        .on_click(cx.listener(move |this, _, _, cx| this.open(&to, cx)))
                        .child(text.clone()),
                )
                .into_any_element()
        }
        Row::Cells { cells, head } => table_row(tokens, cells, *head),
        Row::Code(text) => line(tokens)
            .child(
                div()
                    .w_full()
                    .px(u(tokens.spacing.md))
                    .py(u(tokens.spacing.xs))
                    .rounded(u(tokens.radius.sm))
                    .bg(tokens.colors.element)
                    .font_family(mono.clone())
                    .text_size(u(tokens.font.mono))
                    .whitespace_normal()
                    .child(text.clone()),
            )
            .into_any_element(),
        Row::Heading(text) => div()
            .px(u(tokens.spacing.lg))
            .pt(u(tokens.spacing.md))
            .font_semibold()
            .child(text.clone())
            .into_any_element(),
        Row::Note(text) => line(tokens)
            .text_color(tokens.colors.text_muted)
            .text_size(u(tokens.font.small))
            .child(text.clone())
            .into_any_element(),
    });
    v_flex()
        .child(
            div()
                .px(u(tokens.spacing.lg))
                .pt(u(tokens.spacing.lg))
                .pb(u(tokens.spacing.sm))
                .text_size(u(tokens.font.small))
                .font_semibold()
                .text_color(tokens.colors.text_muted)
                .child(section.title.to_uppercase()),
        )
        .children(rows)
        .into_any_element()
}

/// A padded row container.
fn line(tokens: &Tokens) -> Div {
    h_flex()
        .items_start()
        .gap(u(tokens.spacing.md))
        .px(u(tokens.spacing.lg))
        .py(u(px(1.)))
}

fn label_cell(tokens: &Tokens, label: &str, width: f32) -> Div {
    div()
        .flex_none()
        .w(u(px(width)))
        .truncate()
        .text_color(tokens.colors.text_muted)
        .child(label.to_owned())
}

fn pair(tokens: &Tokens, tones: &ToneColors, label: &str, value: &str, tone: Tone) -> AnyElement {
    line(tokens)
        .child(label_cell(tokens, label, LABEL_WIDTH))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_normal()
                .text_color(tones.of(tone))
                .child(value.to_owned()),
        )
        .into_any_element()
}

/// A used-against-limit bar: the label, a track filled to the ratio, the `used / hard` text.
fn bar(
    tokens: &Tokens,
    tones: &ToneColors,
    ix: usize,
    label: &str,
    text: &str,
    percent: Option<u32>,
    level: QuotaLevel,
) -> AnyElement {
    let tone = match level {
        QuotaLevel::Ok => Tone::Ok,
        QuotaLevel::Warn => Tone::Warn,
        QuotaLevel::Full => Tone::Error,
    };
    let fraction = percent.map_or(0.0, |p| (p.min(100) as f32) / 100.0);
    let shown = match percent {
        Some(p) => format!("{text} ({p}%)"),
        None => text.to_owned(),
    };
    line(tokens)
        .items_center()
        .debug_selector(move || format!("config-bar-{ix}"))
        .child(label_cell(tokens, label, BAR_LABEL_WIDTH))
        .child(
            div()
                .debug_selector(move || format!("config-bar-track-{ix}"))
                .flex_1()
                .min_w(u(px(48.)))
                .h(u(px(6.)))
                .rounded(u(tokens.radius.sm))
                .bg(tokens.colors.element)
                .child(
                    div()
                        .debug_selector(move || format!("config-bar-fill-{ix}"))
                        .h_full()
                        .w(relative(fraction))
                        .rounded(u(tokens.radius.sm))
                        .bg(tones.of(tone)),
                ),
        )
        .child(
            div()
                .flex_none()
                .min_w(u(px(120.)))
                .text_right()
                .text_size(u(tokens.font.small))
                .child(shown),
        )
        .into_any_element()
}

/// One row of a small table: equal-width cells; the head row is muted and bold.
fn table_row(tokens: &Tokens, cells: &[String], head: bool) -> AnyElement {
    line(tokens)
        .when(head, |row| row.pt(u(tokens.spacing.md)))
        .children(cells.iter().map(|cell| {
            let cell = div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(u(tokens.font.small))
                .child(cell.clone());
            if head {
                cell.font_semibold().text_color(tokens.colors.text_muted)
            } else {
                cell
            }
        }))
        .into_any_element()
}
