//! The expanded line (E08-S05): a JSON line's pretty-printed form in a pane under the rows.
//!
//! Rows have one height (the unwrapped list is a `uniform_list`), so a line expands into a pane
//! of its own rather than growing in place: click a JSON row to open it there, click it again (or
//! the pane's close button) to close. The pretty-printed text is built once per line and cached.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use oxikube_domain::log::LogLevel;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::LogView;
use super::rows::level_accent;

/// Height of the pane at 100 % zoom.
const PANE_HEIGHT: f32 = 240.;

/// The line shown in the expanded pane.
pub(crate) struct Expanded {
    pub seq: u64,
    pub level: LogLevel,
    pub time: SharedString,
    pub lines: Arc<[SharedString]>,
}

impl LogView {
    /// Expands the JSON line `seq` into the pane, or closes the pane when it shows that line.
    /// Plain-text lines and lines the buffer no longer holds do nothing.
    pub fn toggle_expanded(&mut self, seq: u64, cx: &mut Context<Self>) {
        if self.expanded.as_ref().is_some_and(|e| e.seq == seq) {
            self.expanded = None;
            cx.notify();
            return;
        }
        if !self.options.json {
            return;
        }
        let Some(session) = &self.session else {
            return;
        };
        let text = session.read(|buffer, _| {
            buffer
                .get_seq(seq)
                .filter(|entry| entry.level.is_some())
                .map(|entry| entry.text.clone())
        });
        let Some(text) = text else {
            return;
        };
        let mut cache = self.records.borrow_mut();
        let (Some(row), Some(lines)) = (cache.row(seq, &text), cache.pretty(seq, &text)) else {
            return;
        };
        drop(cache);
        self.expanded = Some(Expanded {
            seq,
            level: row.level,
            time: row.time.clone(),
            lines,
        });
        cx.notify();
    }

    /// Closes the expanded pane.
    pub fn collapse(&mut self, cx: &mut Context<Self>) {
        if self.expanded.take().is_some() {
            cx.notify();
        }
    }

    /// The seq of the expanded line.
    pub fn expanded_seq(&self) -> Option<u64> {
        self.expanded.as_ref().map(|e| e.seq)
    }

    /// The pretty-printed JSON the pane shows.
    pub fn expanded_text(&self) -> Option<String> {
        let expanded = self.expanded.as_ref()?;
        let lines: Vec<&str> = expanded.lines.iter().map(|l| l.as_ref()).collect();
        Some(lines.join("\n"))
    }

    /// The pane, when a line is expanded.
    pub(crate) fn detail_pane(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let expanded = self.expanded.as_ref()?;
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let title = if expanded.time.is_empty() {
            format!("line {}", expanded.seq)
        } else {
            format!("line {} · {}", expanded.seq, expanded.time)
        };
        let header = h_flex()
            .flex_none()
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.md))
            .py(u(tokens.spacing.sm))
            .items_center()
            .bg(colors.surface)
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                div()
                    .text_size(u(tokens.font.small))
                    .text_color(level_accent(expanded.level, &colors))
                    .child(expanded.level.label().to_uppercase()),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(title),
            )
            .child(
                div()
                    .debug_selector(|| "log-detail-close".to_owned())
                    .child(
                        Button::new("log-detail-close")
                            .label("Close")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(|view, _, _, cx| view.request_collapse(cx))),
                    ),
            );
        let body = v_flex()
            .id("log-detail-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(u(tokens.spacing.md))
            .py(u(tokens.spacing.sm))
            .font_family(cx.mono_font_family())
            .text_size(u(tokens.font.mono))
            .children(expanded.lines.iter().map(|line| json_line(line, &colors)));
        Some(
            v_flex()
                .id("log-detail")
                .debug_selector(|| "log-detail".into())
                .flex_none()
                .h(u(px(PANE_HEIGHT)))
                .border_t_1()
                .border_color(colors.border)
                .bg(colors.background)
                .child(header)
                .child(body)
                .into_any_element(),
        )
    }
}

/// One line of pretty JSON: the key (`"name": `) muted, the value in the text colour.
fn json_line(line: &SharedString, colors: &oxikube_ui::Colors) -> AnyElement {
    let text = line.as_ref();
    let row = h_flex().whitespace_nowrap().flex_none();
    let trimmed = text.trim_start();
    if trimmed.starts_with('"')
        && let Some(split) = text.find("\": ")
    {
        let split = split + 3;
        return row
            .child(
                div()
                    .text_color(colors.text_muted)
                    .child(SharedString::from(text[..split].to_owned())),
            )
            .child(
                div()
                    .text_color(colors.text)
                    .child(SharedString::from(text[split..].to_owned())),
            )
            .into_any_element();
    }
    row.text_color(colors.text)
        .child(line.clone())
        .into_any_element()
}
