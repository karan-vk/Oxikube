//! Drawing the diff tab: the header with its switches, the changed-path summary, and the line
//! diff. Both lists are `uniform_list`s, so only the rows on screen are built.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px, uniform_list,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Selectable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::data::{DiffData, DiffState};
use super::view::ResourceDiffView;
use crate::view::live::{DiffLayout, ROW_HEIGHT, layout_rows, summary};

/// The summary block's height before the UI zoom.
const SUMMARY_HEIGHT: f32 = 120.;

impl Render for ResourceDiffView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let body = match self.state.clone() {
            DiffState::Working(text) => self.message(text.into(), false, cx),
            DiffState::Failed(text) => self.message(text.into(), true, cx),
            DiffState::Ready(data) => self.ready(&data, cx),
        };
        v_flex()
            .id("resource-diff")
            .debug_selector(|| "resource-diff".into())
            .size_full()
            .bg(tokens.colors.surface)
            .text_color(tokens.colors.text)
            .child(self.header(cx))
            .child(body)
    }
}

impl ResourceDiffView {
    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let request = &self.request;
        let side_by_side = self.layout == DiffLayout::SideBySide;
        h_flex()
            .flex_none()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.md))
            .py(u(tokens.spacing.xs))
            .items_center()
            .border_b_1()
            .border_color(tokens.colors.border)
            .text_size(u(tokens.font.small))
            .child(
                div()
                    .id("resource-diff-title")
                    .debug_selector(|| "resource-diff-title".into())
                    .child(self.heading()),
            )
            .child(div().flex_1())
            .child(
                Button::new("resource-diff-status")
                    .label("Status")
                    .tooltip("Compare status too")
                    .ghost()
                    .xsmall()
                    .selected(request.include_status)
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_status(cx))),
            )
            .child(
                Button::new("resource-diff-identity")
                    .label("Name and namespace")
                    .tooltip("Compare metadata.name and metadata.namespace too")
                    .ghost()
                    .xsmall()
                    .selected(request.compare_identity)
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_identity(cx))),
            )
            .child(
                Button::new("resource-diff-layout")
                    .label(if side_by_side {
                        "Side by side"
                    } else {
                        "Inline"
                    })
                    .ghost()
                    .xsmall()
                    .selected(side_by_side)
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_layout(cx))),
            )
            .child(
                Button::new("resource-diff-refresh")
                    .label("Refresh")
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
            )
            .into_any_element()
    }

    /// `staging · ConfigMap shop/web  ↔  production · ConfigMap shop/web`; the references until
    /// the result names the clusters.
    fn heading(&self) -> String {
        match &self.state {
            DiffState::Ready(data) => {
                format!("{}  ↔  {}", data.diff.left.label(), data.diff.right.label())
            }
            _ => format!("{}  ↔  {}", self.request.left, self.request.right),
        }
    }

    fn message(&self, text: SharedString, error: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.tokens().colors;
        div()
            .id("resource-diff-message")
            .debug_selector(|| "resource-diff-message".into())
            .p(u(px(12.)))
            .text_color(if error {
                colors.error
            } else {
                colors.text_muted
            })
            .child(text)
            .into_any_element()
    }

    fn ready(&mut self, data: &Arc<DiffData>, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let family = cx.mono_font_family();
        let lines = data.lines.clone();
        let layout = self.layout;
        let row_count = match layout {
            DiffLayout::SideBySide => lines.side_rows().len(),
            DiffLayout::Inline => lines.inline_rows().len(),
        };
        let diff_list = if lines.is_empty() {
            let text = if data.diff.is_identical() {
                "The two objects are the same."
            } else {
                "Nothing differs once the ignored fields are left out."
            };
            self.message(text.into(), false, cx)
        } else {
            uniform_list(
                "resource-diff-rows",
                row_count,
                cx.processor(move |_, range: Range<usize>, _, cx| {
                    layout_rows(&lines, layout, range, cx)
                }),
            )
            .track_scroll(&self.lines_scroll)
            .flex_1()
            .min_h_0()
            .font_family(family.clone())
            .text_size(u(tokens.font.mono))
            .into_any_element()
        };
        let summary_lines = data.summary.clone();
        let count = summary_lines.len();
        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                h_flex()
                    .flex_none()
                    .px(u(tokens.spacing.md))
                    .py(u(tokens.spacing.xs))
                    .text_size(u(tokens.font.small))
                    .text_color(tokens.colors.text_muted)
                    .child(
                        div()
                            .id("resource-diff-counts")
                            .debug_selector(|| "resource-diff-counts".into())
                            .child(summary(&data.lines)),
                    ),
            )
            .child(
                div()
                    .id("resource-diff-summary")
                    .debug_selector(|| "resource-diff-summary".into())
                    .flex_none()
                    .h(u(px(SUMMARY_HEIGHT)))
                    .border_y_1()
                    .border_color(tokens.colors.border_variant)
                    .child(
                        uniform_list(
                            "resource-diff-summary-rows",
                            count,
                            cx.processor(move |_, range: Range<usize>, _, cx| {
                                summary_rows(&summary_lines, range, cx)
                            }),
                        )
                        .track_scroll(&self.summary_scroll)
                        .size_full()
                        .font_family(family)
                        .text_size(u(tokens.font.mono)),
                    ),
            )
            .child(diff_list)
            .into_any_element()
    }
}

fn summary_rows(lines: &[String], range: Range<usize>, cx: &mut gpui::App) -> Vec<AnyElement> {
    let color = cx.tokens().colors.text;
    range
        .map(|ix| {
            h_flex()
                .w_full()
                .h(u(px(ROW_HEIGHT)))
                .px(u(px(12.)))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(color)
                .child(lines[ix].clone())
                .into_any_element()
        })
        .collect()
}
