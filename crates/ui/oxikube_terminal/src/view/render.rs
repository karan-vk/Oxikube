//! Drawing a [`TerminalView`]: the element over the running session, a line while the process
//! starts or when it could not, and the exit line once it ended (the screen stays readable).

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use oxikube_ports::ExitStatus;
use oxikube_ui::button::Button;
use oxikube_ui::layout::v_flex;
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::TerminalView;
use super::terminal_view::Phase;
use crate::element::{PathLinks, TerminalElement};

impl TerminalView {
    /// Paths in a local shell's output are links (relative ones against its start directory); a
    /// pod's paths are inside the container.
    fn path_links(&self) -> PathLinks {
        if self.descriptor.is_local() {
            PathLinks::Local {
                base: self.descriptor.cwd().map(ToOwned::to_owned),
            }
        } else {
            PathLinks::Off
        }
    }

    fn message(
        &self,
        selector: &'static str,
        text: SharedString,
        cx: &Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        div()
            .debug_selector(move || selector.into())
            .size_full()
            .p(u(tokens.spacing.md))
            .bg(tokens.colors.background)
            .text_color(tokens.colors.text_muted)
            .child(text)
            .into_any_element()
    }

    /// The terminal that could not start: why, and a Retry that starts it again (a pod session
    /// is asked for again through its command, so the read-only policy applies to the retry too).
    fn failed(&self, reason: &str, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let what = if self.descriptor.is_local() {
            "The terminal could not start".to_owned()
        } else {
            format!("Could not open a session in {}", self.title())
        };
        v_flex()
            .debug_selector(|| "terminal-failed".into())
            .size_full()
            .gap(u(tokens.spacing.md))
            .p(u(tokens.spacing.md))
            .bg(tokens.colors.background)
            .text_color(tokens.colors.text_muted)
            .child(div().child(format!("{what}: {reason}")))
            .child(
                div().debug_selector(|| "terminal-retry".into()).child(
                    Button::new("terminal-retry")
                        .small()
                        .label("Retry")
                        .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
                ),
            )
            .into_any_element()
    }
}

/// The line under a terminal whose process ended.
pub fn describe_exit(status: &ExitStatus) -> String {
    match (&status.code, &status.signal) {
        (Some(code), _) => format!("Process exited with code {code}"),
        (None, Some(signal)) => format!("Process ended by signal {signal}"),
        (None, None) => "Process ended".to_owned(),
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.terminal().is_some();
        let body = match &self.phase {
            Phase::Running(state) => {
                let mut element = TerminalElement::new(state, &self.element, &self.focus)
                    .path_links(self.path_links());
                if let Some(dispatcher) = self.services.dispatcher() {
                    element = element.dispatcher(dispatcher.clone());
                }
                if let Some(confirm) = self.services.paste_confirm() {
                    element = element.paste_confirm(confirm.clone());
                }
                element.into_any_element()
            }
            Phase::Starting => {
                let text = format!("Starting {}…", self.title()).into();
                self.message("terminal-starting", text, cx)
            }
            Phase::Failed(reason) => self.failed(reason, cx),
            Phase::Closed => div().into_any_element(),
        };
        let exit = self.exit_status(cx).map(describe_exit);
        let tokens = cx.tokens();
        div()
            .id("terminal-view")
            .relative()
            .size_full()
            // The element tracks the focus handle itself; without it the body does, so the tab
            // takes focus while the process starts.
            .when(!running, |this| this.track_focus(&self.focus))
            .child(body)
            .when_some(exit, |this, line| {
                this.child(
                    div()
                        .debug_selector(|| "terminal-exited".into())
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .px(u(tokens.spacing.md))
                        .py(u(px(2.)))
                        .bg(tokens.colors.surface)
                        .text_color(tokens.colors.text_muted)
                        .text_size(u(tokens.font.small))
                        .child(line),
                )
            })
    }
}
