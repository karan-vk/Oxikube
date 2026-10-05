//! The content of the main window: the title bar over the [`Workspace`].
//!
//! This is the view the `Root` hosts. The workspace fills the body (docks and centre panes); the
//! status bar arrives with E05-S10.

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Window, div, px,
};
use oxikube_ui::{ActiveTokens as _, layout::v_flex, title_bar::TitleBar};

use super::options::WINDOW_TITLE;
use crate::workspace::Workspace;

/// Root content view of the main window.
pub struct MainView {
    workspace: Entity<Workspace>,
}

impl MainView {
    /// The title bar over a new, empty workspace.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            workspace: cx.new(|cx| Workspace::new(window, cx)),
        }
    }

    /// The window's workspace.
    pub fn workspace(&self) -> &Entity<Workspace> {
        &self.workspace
    }
}

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        v_flex()
            .id("main-view")
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                TitleBar::new().child(
                    div()
                        .id("window-title")
                        .debug_selector(|| "window-title".to_owned())
                        .text_sm()
                        .text_color(colors.text_muted)
                        .px(px(8.))
                        .child(WINDOW_TITLE),
                ),
            )
            .child(
                div()
                    .id("main-body")
                    .flex_1()
                    .min_h_0()
                    .child(self.workspace.clone()),
            )
    }
}
