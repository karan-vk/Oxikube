//! The content of the main window: the title bar over an (as yet empty) themed body.
//!
//! This is the view the `Root` hosts. E05-S04 replaces the empty body with the workspace
//! (docks, pane group, status bar); the title bar stays.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, px,
};
use oxikube_ui::{ActiveTokens as _, layout::v_flex, title_bar::TitleBar};

use super::options::WINDOW_TITLE;

/// Root content view of the main window.
pub struct MainView;

impl MainView {
    /// Creates the view.
    pub fn new() -> Self {
        Self
    }
}

impl Default for MainView {
    fn default() -> Self {
        Self::new()
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
            .child(div().id("main-body").flex_1())
    }
}
