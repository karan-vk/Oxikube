//! A structured line as a row (E08-S05): chevron, level chip, time, message and the other fields
//! collapsed to `key=value`. Clicking it expands the line into the pane under the rows.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};
use oxikube_domain::log::LogLevel;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, u};

use super::json::JsonColumns;
use super::rows::level_accent;
use super::{LogView, NOWRAP_CHARS};

/// Width of the level chip and the time column at 100 % zoom: fixed, so the messages line up.
const CHIP_WIDTH: f32 = 48.;
const TIME_WIDTH: f32 = 92.;

impl LogView {
    /// The row of a structured line, built on `base` (the row's shared frame).
    #[allow(clippy::too_many_arguments, reason = "everything one row draws")]
    pub(super) fn json_row(
        &self,
        base: Div,
        seq: u64,
        ts: Option<SharedString>,
        record: &JsonColumns,
        expanded: bool,
        wrap: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let accent = level_accent(record.level, &colors);
        let chevron = div()
            .flex_none()
            .w(u(px(10.)))
            .text_color(colors.text_muted)
            .child(if expanded { "▾" } else { "▸" });
        let chip = div()
            .flex_none()
            .w(u(px(CHIP_WIDTH)))
            .text_center()
            .rounded(u(tokens.radius.sm))
            .bg(accent.opacity(0.16))
            .text_color(accent)
            .font_weight(chip_weight(record.level))
            .child(chip_label(record.level));
        let time = div()
            .flex_none()
            .w(u(px(TIME_WIDTH)))
            .whitespace_nowrap()
            .text_color(colors.text_muted)
            .child(record.time.clone());
        let message = message_text(&record.message, wrap);
        let body = if wrap {
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().text_color(colors.text).child(message))
                .when(!record.summary.is_empty(), |column| {
                    column.child(
                        div()
                            .text_color(colors.text_muted)
                            .child(record.summary.clone()),
                    )
                })
                .into_any_element()
        } else {
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(u(tokens.spacing.md))
                .overflow_hidden()
                .child(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .text_color(colors.text)
                        .child(message),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_color(colors.text_muted)
                        .child(record.summary.clone()),
                )
                .into_any_element()
        };
        let ts = ts.map(|ts| {
            div()
                .flex_none()
                .whitespace_nowrap()
                .text_color(colors.text_muted)
                .child(ts)
        });
        base.id(("log-row", seq))
            .debug_selector(move || format!("log-row:{seq}"))
            .cursor_pointer()
            .when(expanded, |row| row.bg(colors.element_selected))
            .hover(|row| row.bg(colors.element_hover))
            .on_click(cx.listener(move |view, _, _, cx| view.request_toggle_line(seq, cx)))
            .child(chevron)
            .children(ts)
            .child(chip)
            .child(time)
            .child(body)
            .into_any_element()
    }
}

/// Error and fatal chips are bold; every other level, and the dash of a line with none, is not.
/// (`LogLevel` orders `Unknown` last, so a `>=` comparison would count it as the worst.)
fn chip_weight(level: LogLevel) -> FontWeight {
    if matches!(level, LogLevel::Error | LogLevel::Fatal) {
        FontWeight::BOLD
    } else {
        FontWeight::MEDIUM
    }
}

/// `INFO`, `WARN`, ...; a structured line with no level shows a dash.
fn chip_label(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Trace => "TRACE",
        LogLevel::Debug => "DEBUG",
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
        LogLevel::Fatal => "FATAL",
        LogLevel::Unknown => "—",
    }
}

/// The message a row draws: all of it wrapped, the first [`NOWRAP_CHARS`] bytes unwrapped.
fn message_text(message: &SharedString, wrap: bool) -> SharedString {
    if wrap || message.len() <= NOWRAP_CHARS {
        return message.clone();
    }
    let mut end = NOWRAP_CHARS;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    SharedString::from(message[..end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_error_and_fatal_chips_are_bold() {
        for level in [LogLevel::Error, LogLevel::Fatal] {
            assert_eq!(chip_weight(level), FontWeight::BOLD, "{level:?}");
        }
        for level in [
            LogLevel::Trace,
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Unknown,
        ] {
            assert_eq!(chip_weight(level), FontWeight::MEDIUM, "{level:?}");
        }
    }
}
