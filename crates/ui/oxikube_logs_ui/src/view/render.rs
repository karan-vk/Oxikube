//! The frame: key context and actions, the toolbar, the virtualised rows and the "N new lines"
//! pill.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, list, px, uniform_list,
};
use oxikube_domain::log::LogRange;
use oxikube_keymap::KeyContextual as _;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::v_flex;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::LogView;
use super::actions::{
    Copy, Head, Mark, Since1h, Since1m, Since5m, Since15m, Since30m, Tail, ToggleAutoscroll,
    ToggleFullscreen, TogglePrevious, ToggleTimestamps, ToggleWrap,
};
use super::text::group;

impl Render for LogView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        self.rows_built = 0;
        v_flex()
            .id("log-view")
            .debug_selector(|| "log-view".into())
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .on_action(cx.listener(|v, _: &Tail, _, cx| v.request_range(LogRange::Tail, cx)))
            .on_action(cx.listener(|v, _: &Head, _, cx| v.request_range(LogRange::Head, cx)))
            .on_action(cx.listener(|v, _: &Since1m, _, cx| v.request_range(LogRange::Last1m, cx)))
            .on_action(cx.listener(|v, _: &Since5m, _, cx| v.request_range(LogRange::Last5m, cx)))
            .on_action(cx.listener(|v, _: &Since15m, _, cx| v.request_range(LogRange::Last15m, cx)))
            .on_action(cx.listener(|v, _: &Since30m, _, cx| v.request_range(LogRange::Last30m, cx)))
            .on_action(cx.listener(|v, _: &Since1h, _, cx| v.request_range(LogRange::Last1h, cx)))
            .on_action(cx.listener(|v, _: &ToggleAutoscroll, _, cx| v.request_autoscroll(cx)))
            .on_action(cx.listener(|v, _: &ToggleWrap, _, cx| v.request_wrap(cx)))
            .on_action(cx.listener(|v, _: &ToggleTimestamps, _, cx| v.request_timestamps(cx)))
            .on_action(cx.listener(|v, _: &TogglePrevious, _, cx| v.request_previous(cx)))
            .on_action(cx.listener(|v, _: &ToggleFullscreen, _, cx| v.request_fullscreen(cx)))
            .on_action(cx.listener(|v, _: &Mark, _, cx| v.mark(cx)))
            .on_action(cx.listener(|v, _: &Copy, _, cx| v.copy(cx)))
            .size_full()
            .bg(tokens.colors.background)
            .text_color(tokens.colors.text)
            .child(self.toolbar(cx))
            .child(self.body(cx))
    }
}

impl LogView {
    fn body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = if self.options.wrap {
            list(
                self.list.clone(),
                cx.processor(|view, ix, _, cx| view.render_wrapped(ix, cx)),
            )
            .size_full()
            .into_any_element()
        } else {
            uniform_list(
                "log-rows",
                self.window.row_count(),
                cx.processor(|view, range, _, cx| view.render_rows(range, cx)),
            )
            .track_scroll(&self.scroll)
            .size_full()
            .into_any_element()
        };
        div()
            .id("log-body")
            .debug_selector(|| "log-body".into())
            .relative()
            .flex_1()
            .min_h_0()
            .on_scroll_wheel(cx.listener(Self::on_wheel))
            .child(rows)
            .children(self.pill(cx))
    }

    /// "N new lines": shown while autoscroll is paused and lines arrived; a click follows again.
    fn pill(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let count = self.new_lines();
        if count == 0 {
            return None;
        }
        let tokens = cx.tokens();
        let label = if count == 1 {
            "1 new line".to_owned()
        } else {
            format!("{} new lines", group(count))
        };
        let selector = format!("log-new-lines:{count}");
        Some(
            div()
                .absolute()
                .bottom(u(tokens.spacing.lg))
                .right(u(tokens.spacing.xl))
                .debug_selector(move || selector)
                .child(
                    Button::new("log-new-lines")
                        .primary()
                        .small()
                        .icon(Icon::new(IconName::ArrowDown).size(u(px(14.))))
                        .label(label)
                        .on_click(cx.listener(|view, _, _, cx| view.jump_to_newest(cx))),
                ),
        )
    }
}
