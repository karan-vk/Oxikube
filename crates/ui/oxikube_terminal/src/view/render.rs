//! Drawing a [`TerminalView`]: the element over the running session, a line while the process
//! starts, and above the screen the banner of a session that dropped, ended or could not start
//! (the screen stays readable, dimmed when input no longer reaches the process).

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, prelude::FluentBuilder as _,
};
use oxikube_ports::ExitStatus;
use oxikube_ui::layout::v_flex;
use oxikube_ui::{ActiveTokens as _, u};

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
        // A session that dropped or ended keeps its screen, dimmed: keystrokes go nowhere now.
        let dimmed = running && !self.lifecycle.accepts_input();
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
            // The banner above says why; the body is only the backdrop.
            Phase::Failed(_) => self.message("terminal-failed", SharedString::default(), cx),
            Phase::Closed => div().into_any_element(),
        };
        let banner = self.banner().map(|banner| self.banner_strip(banner, cx));
        let tokens = cx.tokens();
        v_flex()
            .id("terminal-view")
            .size_full()
            // The element tracks the focus handle itself; without it the body does, so the tab
            // takes focus while the process starts.
            .when(!running, |this| this.track_focus(&self.focus))
            .children(banner)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(body)
                    .when(dimmed, |this| {
                        // No listeners: the screen stays selectable and scrollable under it.
                        this.child(
                            div()
                                .debug_selector(|| "terminal-dimmed".into())
                                .absolute()
                                .inset_0()
                                .bg(tokens.colors.background.opacity(0.35)),
                        )
                    }),
            )
    }
}
