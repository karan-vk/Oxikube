//! Drawing the containers sub-table: a heading, a header row and one uniform row per container,
//! in fixed-width columns that scroll sideways in a narrow drawer.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Tokens, u};

use super::rows::{COLUMNS, ContainerRow};
use super::{ContainersPanel, MAX_HEIGHT};
use crate::table::ToneColors;

/// One row's height, unscaled: every row is one line.
const ROW_HEIGHT: f32 = 24.;

impl Render for ContainersPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.doc.is_none() {
            return div().into_any_element();
        }
        let tokens = cx.tokens();
        let tones = ToneColors::current(cx);
        let width: f32 = COLUMNS.iter().map(|c| c.2).sum::<f32>()
            + COLUMNS.len() as f32 * f32::from(tokens.spacing.md);
        let heading = div()
            .debug_selector(|| "pod-containers-heading".to_owned())
            .px(u(tokens.spacing.lg))
            .pt(u(tokens.spacing.lg))
            .pb(u(tokens.spacing.sm))
            .text_size(u(tokens.font.small))
            .font_semibold()
            .text_color(tokens.colors.text_muted)
            .child(format!("CONTAINERS ({})", self.rows.len()));
        let table = v_flex().w(u(px(width))).child(header(&tokens)).children(
            self.rows
                .iter()
                .map(|row| row_element(&tokens, &tones, row)),
        );
        v_flex()
            .debug_selector(|| "pod-containers".to_owned())
            .w_full()
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .pb(u(tokens.spacing.sm))
            .child(heading)
            .child(
                div()
                    .id("pod-containers-scroll")
                    .w_full()
                    .max_h(u(px(MAX_HEIGHT)))
                    .overflow_scroll()
                    .px(u(tokens.spacing.lg))
                    .child(table),
            )
            .into_any_element()
    }
}

fn header(tokens: &Tokens) -> AnyElement {
    h_flex()
        .h(u(px(ROW_HEIGHT)))
        .gap(u(tokens.spacing.md))
        .border_b_1()
        .border_color(tokens.colors.border_variant)
        .text_size(u(tokens.font.small))
        .text_color(tokens.colors.text_muted)
        .children(COLUMNS.iter().map(|&(id, title, width, right)| {
            cell(width, right)
                .debug_selector(move || format!("pod-containers-header-{id}"))
                .child(title)
        }))
        .into_any_element()
}

fn row_element(tokens: &Tokens, tones: &ToneColors, row: &ContainerRow) -> AnyElement {
    let index = row.index;
    let cells = row.cells();
    h_flex()
        .debug_selector(move || format!("pod-container-{index}"))
        .h(u(px(ROW_HEIGHT)))
        .gap(u(tokens.spacing.md))
        .children(
            COLUMNS
                .iter()
                .zip(cells)
                .map(|(&(id, _, width, right), text)| {
                    let colour = match (id, row.usage_tone(id)) {
                        ("state", _) => tones.of(row.state_tone),
                        (_, Some(tone)) => tones.of(tone),
                        ("image" | "probes" | "ports" | "cpu" | "memory", _) => {
                            tokens.colors.text_muted
                        }
                        _ => tokens.colors.text,
                    };
                    let cell = cell(width, right)
                        .debug_selector(move || format!("pod-container-{index}-{id}"))
                        .text_color(colour);
                    if id == "name" {
                        cell.child(name(tokens, &text, row.marker))
                            .into_any_element()
                    } else {
                        cell.child(text).into_any_element()
                    }
                }),
        )
        .into_any_element()
}

/// The name, followed by the type marker (`init`, `sidecar`, `ephemeral`) when there is one.
fn name(tokens: &Tokens, name: &SharedString, marker: &'static str) -> AnyElement {
    h_flex()
        .gap(u(tokens.spacing.xs))
        .min_w_0()
        .child(div().min_w_0().truncate().child(name.clone()))
        .when(!marker.is_empty(), |el| {
            el.child(
                div()
                    .flex_none()
                    .px(u(tokens.spacing.xs))
                    .rounded(u(tokens.radius.sm))
                    .bg(tokens.colors.element)
                    .text_size(u(tokens.font.small))
                    .text_color(tokens.colors.text_muted)
                    .child(marker),
            )
        })
        .into_any_element()
}

/// A cell of `width` unscaled pixels: one line, cut with an ellipsis.
fn cell(width: f32, right: bool) -> gpui::Div {
    div()
        .flex_none()
        .w(u(px(width)))
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .when(right, |el| el.text_right())
}
