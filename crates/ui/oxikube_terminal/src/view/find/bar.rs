//! The search bar: the field, previous and next, the count (or the pattern's error) and close.
//! Every control sends its `terminal::Search*` command through the same path as the keys.

use gpui::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Styled as _, Window, div, px,
};
use oxikube_domain::command::Command;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::input::{Input, InputEvent, InputState};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use crate::view::TerminalView;

/// The words under the field: `3 of 41`, `No matches`, or why the pattern is refused.
pub(super) fn status_text(count: usize, current: Option<usize>, pattern_empty: bool) -> String {
    match (count, current) {
        _ if pattern_empty => String::new(),
        (0, _) => "No matches".to_owned(),
        (count, Some(current)) => format!("{} of {count}", current + 1),
        (count, None) => format!("{count} matches"),
    }
}

impl TerminalView {
    /// Makes the field the first time the bar is shown, with the text the last search had.
    pub(super) fn ensure_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(input) = &self.find.input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find (regex)"));
        self.find.subscription = Some(cx.subscribe_in(&input, window, Self::on_search_input));
        self.find.input = Some(input.clone());
        input
    }

    fn on_search_input(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let text = input.read(cx).value().to_string();
                self.search_edited(&text, cx);
            }
            InputEvent::PressEnter { shift, .. } => {
                let command = if *shift {
                    Command::TerminalSearchPrevious
                } else {
                    Command::TerminalSearchNext
                };
                self.dispatch_search(command, window, cx);
            }
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    /// Sends `command` through the bus (the services' dispatcher), like the key bindings do;
    /// without a dispatcher (a bare test) it runs directly.
    pub(super) fn dispatch_search(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(dispatcher) = self.services.dispatcher().cloned() {
            dispatcher.dispatch(command, cx);
            return;
        }
        match command {
            Command::TerminalSearch => self.open_search(window, cx),
            Command::TerminalSearchNext => self.search_next(cx),
            Command::TerminalSearchPrevious => self.search_previous(cx),
            Command::TerminalSearchClose => self.close_search(window, cx),
            _ => {}
        }
    }

    /// The bar, when it is open.
    pub(in crate::view) fn search_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.find.is_open() {
            return None;
        }
        let input = self.ensure_search_input(window, cx);
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let failed = self.find.error.is_some();
        let status: SharedString = match &self.find.error {
            Some(error) => error.clone(),
            None => status_text(
                self.find.matches.len(),
                self.find.current,
                self.find.pattern.is_empty(),
            )
            .into(),
        };
        Some(
            h_flex()
                .id("terminal-search")
                .debug_selector(|| "terminal-search".into())
                .key_context("TerminalSearch")
                .flex_none()
                .gap(u(tokens.spacing.sm))
                .px(u(tokens.spacing.md))
                .py(u(tokens.spacing.sm))
                .items_center()
                .bg(colors.surface)
                .border_b_1()
                .border_color(colors.border_variant)
                .child(Icon::new(IconName::Search).size(u(px(14.))))
                .child(
                    div()
                        .debug_selector(|| "terminal-search-input".into())
                        .w(u(px(280.)))
                        .child(Input::new(&input).xsmall()),
                )
                .child(self.search_step(
                    "terminal-search-prev",
                    IconName::ArrowUp,
                    Command::TerminalSearchPrevious,
                    cx,
                ))
                .child(self.search_step(
                    "terminal-search-next",
                    IconName::ArrowDown,
                    Command::TerminalSearchNext,
                    cx,
                ))
                .child(
                    div()
                        .debug_selector(|| "terminal-search-status".into())
                        .max_w(u(px(420.)))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(u(tokens.font.small))
                        .text_color(if failed {
                            colors.error
                        } else {
                            colors.text_muted
                        })
                        .child(status),
                )
                .child(div().flex_1())
                .child(self.search_step(
                    "terminal-search-close",
                    IconName::X,
                    Command::TerminalSearchClose,
                    cx,
                ))
                .into_any_element(),
        )
    }

    fn search_step(
        &self,
        id: &'static str,
        icon: IconName,
        command: Command,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .debug_selector(move || id.to_owned())
            .child(
                Button::new(id)
                    .icon(Icon::new(icon).size(u(px(14.))))
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.dispatch_search(command.clone(), window, cx);
                    })),
            )
            .into_any_element()
    }
}
