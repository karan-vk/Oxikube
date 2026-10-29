//! The frame of the panel: the header with the source bar, the body of the stage shown, and the
//! footer with the actions.

mod footer;
mod header;
mod load;
mod look;
mod table;

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div,
};
use oxikube_keymap::KeyContextual as _;
use oxikube_ui::layout::v_flex;
use oxikube_ui::{ActiveTokens as _, u};

use super::keys::{
    OpenInEditor, SelectFirst, SelectLast, SelectNext, SelectPrevious, ToggleErrorsOnly,
    ToggleExpand,
};
use super::model::Stage;
use super::view::ApplyView;

pub use look::{ROW_HEIGHT, status_look};

impl Render for ApplyView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let body = match self.model.stage() {
            Stage::Load => self.load_body(cx),
            Stage::Review | Stage::Result => self.table_body(window, cx),
        };
        v_flex()
            .id("apply-panel")
            .debug_selector(|| "apply-panel".into())
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| this.step(isize::MIN, cx)))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.step(isize::MAX, cx)))
            .on_action(cx.listener(|this, _: &ToggleExpand, _, cx| this.toggle_selected(cx)))
            .on_action(cx.listener(|this, _: &ToggleErrorsOnly, _, cx| this.toggle_errors_only(cx)))
            .on_action(cx.listener(|this, _: &OpenInEditor, window, cx| {
                this.open_selected_in_editor(window, cx)
            }))
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .child(self.header(cx))
            .child(div().flex_1().min_h_0().child(body))
            .child(self.footer(cx))
    }
}
