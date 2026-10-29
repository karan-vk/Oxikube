//! The Review and Result view: the plan as a virtualised table. One row per object (icon, kind,
//! name, namespace, message, time), and under an expanded failed row the server's full message
//! and field paths, each on a line of the same height.

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px, uniform_list,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, Tokens, u};

use super::look::{ROW_HEIGHT, status_look, status_message};
use crate::apply::model::{Line, Stage, details};
use crate::apply::view::ApplyView;

impl ApplyView {
    pub(super) fn table_body(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let count = self.model.lines().len();
        let scroll = self.table_scroll.clone();
        let empty = if count == 0 {
            Some(if self.model.errors_only() {
                "No failures."
            } else {
                "Nothing to show."
            })
        } else {
            None
        };
        v_flex()
            .debug_selector(|| "apply-table".into())
            .size_full()
            .child(self.table_header(&tokens))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .when_some(empty, |list, text| {
                        list.child(
                            div()
                                .p(u(tokens.spacing.xl))
                                .text_color(tokens.colors.text_muted)
                                .child(text),
                        )
                    })
                    .when(count > 0, |list| {
                        list.child(
                            uniform_list(
                                "apply-rows",
                                count,
                                cx.processor(|this, range: Range<usize>, _, cx| {
                                    this.table_rows(range, cx)
                                }),
                            )
                            .track_scroll(&scroll)
                            .size_full(),
                        )
                    }),
            )
            .into_any_element()
    }

    fn table_header(&self, tokens: &Tokens) -> AnyElement {
        let colors = tokens.colors;
        let time = if self.model.stage() == Stage::Result {
            "Time"
        } else {
            "Dry run"
        };
        h_flex()
            .flex_none()
            .h(u(px(ROW_HEIGHT)))
            .px(u(tokens.spacing.xl))
            .gap(u(tokens.spacing.md))
            .items_center()
            .border_b_1()
            .border_color(colors.border_variant)
            .text_color(colors.text_muted)
            .text_size(u(tokens.font.small))
            .child(div().flex_none().w(u(px(18.))))
            .child(div().flex_none().w(u(px(150.))).child("Kind"))
            .child(div().flex_none().w(u(px(220.))).child("Name"))
            .child(div().flex_none().w(u(px(150.))).child("Namespace"))
            .child(div().flex_1().min_w_0().child("Message"))
            .child(div().flex_none().w(u(px(64.))).child(time))
            .child(div().flex_none().w(u(px(52.))))
            .into_any_element()
    }

    fn table_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let tokens = cx.tokens();
        let lines: Vec<Line> = self
            .model
            .lines()
            .get(range.clone())
            .map(<[Line]>::to_vec)
            .unwrap_or_default();
        range
            .zip(lines)
            .map(|(position, line)| match line {
                Line::Row(index) => self.row(position, index, &tokens, cx),
                Line::Detail(index, text) => detail_line(position, index, text, &tokens),
            })
            .collect()
    }

    fn row(
        &self,
        position: usize,
        index: usize,
        tokens: &Tokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = tokens.colors;
        let Some(row) = self.model.rows().get(index) else {
            return div().h(u(px(ROW_HEIGHT))).into_any_element();
        };
        let status = row.shown();
        let (icon, tone) = status_look(status, &colors);
        let word = status.map_or("waiting", |status| status.word());
        let selected = self.model.selected() == Some(index);
        let expandable = !details(row).is_empty();
        let expanded = self.model.is_expanded(index);
        let elapsed = row
            .elapsed
            .map(|elapsed| format!("{:.1}s", elapsed.as_secs_f32()))
            .unwrap_or_default();
        let namespace = row.namespace.clone();
        h_flex()
            .id(("apply-row", position))
            .debug_selector(move || format!("apply-row-{index}"))
            .h(u(px(ROW_HEIGHT)))
            .w_full()
            .px(u(tokens.spacing.xl))
            .gap(u(tokens.spacing.md))
            .items_center()
            .text_size(u(tokens.font.small))
            .when(selected, |row| row.bg(colors.element_selected))
            .hover(|row| row.bg(colors.element_hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select(index, cx);
                this.toggle_expanded(index, cx);
            }))
            .child(
                div()
                    .debug_selector(move || format!("apply-status-{index}-{word}"))
                    .flex_none()
                    .w(u(px(18.)))
                    .child(Icon::new(icon).size(u(px(14.))).color(tone)),
            )
            .child(
                div()
                    .flex_none()
                    .w(u(px(150.)))
                    .truncate()
                    .child(row.kind.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .w(u(px(220.)))
                    .truncate()
                    .text_color(colors.text)
                    .child(row.name.clone()),
            )
            .child(match namespace {
                Some(namespace) => div()
                    .flex_none()
                    .w(u(px(150.)))
                    .truncate()
                    .child(namespace)
                    .into_any_element(),
                None => div()
                    .debug_selector(move || format!("apply-cluster-scoped-{index}"))
                    .flex_none()
                    .w(u(px(150.)))
                    .text_color(colors.warning)
                    .child("cluster-scoped")
                    .into_any_element(),
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(u(tokens.spacing.xs))
                    .items_center()
                    .when(expandable, |cell| {
                        cell.child(
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(u(px(12.)))
                            .color(colors.text_muted),
                        )
                    })
                    .child(
                        div()
                            .truncate()
                            .text_color(if row.failed() {
                                tone
                            } else {
                                colors.text_muted
                            })
                            .child(SharedString::from(status_message(status))),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .w(u(px(64.)))
                    .text_color(colors.text_muted)
                    .child(elapsed),
            )
            .child(
                h_flex()
                    .debug_selector(move || format!("apply-open-{index}"))
                    .flex_none()
                    .w(u(px(52.)))
                    .justify_center()
                    .child(
                        Button::new(("apply-open", position))
                            .xsmall()
                            .ghost()
                            .icon(Icon::new(IconName::ExternalLink).size(u(px(12.))))
                            .tooltip(format!("Open in editor at line {}", row.line))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.select(index, cx);
                                this.open_selected_in_editor(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}

fn detail_line(position: usize, index: usize, text: String, tokens: &Tokens) -> AnyElement {
    let colors = tokens.colors;
    h_flex()
        .id(("apply-detail", position))
        .debug_selector(move || format!("apply-detail-{index}-{position}"))
        .h(u(px(ROW_HEIGHT)))
        .w_full()
        .pl(u(px(18. + 150. + 12. + 12.)))
        .pr(u(tokens.spacing.xl))
        .items_center()
        .bg(colors.surface)
        .text_size(u(tokens.font.small))
        .font_family("monospace")
        .text_color(
            if text.starts_with("field:") || text.starts_with("conflict:") {
                colors.warning
            } else {
                colors.text
            },
        )
        .child(SharedString::from(text))
        .into_any_element()
}
