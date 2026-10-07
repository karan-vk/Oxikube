//! The frame: key context and actions, the toolbar, the virtualised rows and the "N new lines"
//! pill.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, list, px, uniform_list,
};
use oxikube_domain::log::{LogRange, LogSaveScope};
use oxikube_keymap::KeyContextual as _;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::v_flex;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::LogView;
use super::actions::{
    Clear, ClearSelection, CloseSearch, Copy, Find, FollowReplacement, Head, Mark, NextMatch,
    PreviousMatch, Reconnect, SaveAll, SaveVisible, SendToAgent, Since1h, Since1m, Since5m,
    Since15m, Since30m, Tail, TailInTerminal, ToggleAutoscroll, ToggleCase, ToggleFilterMode,
    ToggleFullscreen, ToggleInverse, ToggleJsonMode, TogglePrevious, ToggleTimestamps, ToggleWrap,
};
use super::text::group;

impl Render for LogView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .on_action(cx.listener(|v, _: &ToggleJsonMode, _, cx| v.request_json_mode(cx)))
            .on_action(cx.listener(|v, _: &TogglePrevious, _, cx| v.request_previous(cx)))
            .on_action(cx.listener(|v, _: &ToggleFullscreen, _, cx| v.request_fullscreen(cx)))
            .on_action(cx.listener(|v, _: &Find, _, cx| v.request_find(cx)))
            .on_action(cx.listener(|v, _: &NextMatch, _, cx| v.request_next_match(cx)))
            .on_action(cx.listener(|v, _: &PreviousMatch, _, cx| v.request_previous_match(cx)))
            .on_action(cx.listener(|v, _: &ToggleCase, _, cx| v.request_toggle_case(cx)))
            .on_action(cx.listener(|v, _: &ToggleInverse, _, cx| v.request_toggle_inverse(cx)))
            .on_action(
                cx.listener(|v, _: &ToggleFilterMode, _, cx| v.request_toggle_filter_mode(cx)),
            )
            .on_action(cx.listener(|v, _: &CloseSearch, _, cx| v.request_close_search(cx)))
            .on_action(cx.listener(|v, _: &Mark, _, cx| v.request_mark(cx)))
            .on_action(cx.listener(|v, _: &Copy, _, cx| v.request_copy(cx)))
            .on_action(cx.listener(|v, _: &SendToAgent, _, cx| v.request_send_to_agent(cx)))
            .on_action(cx.listener(|v, _: &TailInTerminal, _, cx| v.request_tail_in_terminal(cx)))
            .on_action(cx.listener(|v, _: &Clear, _, cx| v.request_clear(cx)))
            .on_action(cx.listener(|v, _: &SaveAll, _, cx| v.request_save(LogSaveScope::All, cx)))
            .on_action(
                cx.listener(|v, _: &SaveVisible, _, cx| v.request_save(LogSaveScope::Visible, cx)),
            )
            .on_action(cx.listener(|v, _: &ClearSelection, _, cx| v.clear_selection(cx)))
            .on_action(cx.listener(|v, _: &Reconnect, _, cx| v.request_reconnect(cx)))
            .on_action(
                cx.listener(|v, _: &FollowReplacement, _, cx| v.request_follow_replacement(cx)),
            )
            .size_full()
            .bg(tokens.colors.background)
            .text_color(tokens.colors.text)
            .child(self.toolbar(cx))
            .children(self.recovery_strip(cx))
            .children(self.crash_hint(cx))
            .children(self.banner(cx))
            .children(self.search_bar(window, cx))
            .children(self.level_bar(cx))
            .child(self.body(cx))
            .children(self.detail_pane(cx))
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
            // The slack above keeps the oldest row on screen whole (`snap`).
            div()
                .size_full()
                .pt(self.snap.slack(self.window.row_count(), self.row_height()))
                .child(
                    div()
                        .size_full()
                        .debug_selector(|| "log-rows".into())
                        .child(
                            uniform_list(
                                "log-rows",
                                self.window.row_count(),
                                cx.processor(|view, range, _, cx| view.render_rows(range, cx)),
                            )
                            .track_scroll(&self.scroll)
                            .size_full(),
                        ),
                )
                .into_any_element()
        };
        div()
            .id("log-body")
            .debug_selector(|| "log-body".into())
            .relative()
            .flex_1()
            .min_h_0()
            .on_scroll_wheel(cx.listener(Self::on_wheel))
            .child(self.body_probe(cx))
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
