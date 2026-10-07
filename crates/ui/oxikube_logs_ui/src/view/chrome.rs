//! What a row carries besides its words (E08-S06): the gutter bar of a marked line, the
//! selection's background, and the pointer handlers that select.

use gpui::{
    Context, Div, InteractiveElement as _, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement as _, Styled as _, Window, div, px,
};
use oxikube_ui::{ActiveTokens as _, u};

use super::LogView;

impl LogView {
    /// `row` (a line's or a structured line's) with the user's marks and selection on it: a bar
    /// over the row's left padding for a marked line (so marking moves nothing), the selection
    /// colour behind a selected one, and a click, shift-click or drag over it that selects its
    /// line (by seq).
    pub(super) fn row_chrome(
        &self,
        row: Div,
        seq: u64,
        marked: bool,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let colors = cx.tokens().colors;
        let gutter = marked.then(|| {
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(u(px(3.)))
                .bg(colors.warning)
                .debug_selector(move || format!("log-mark:{seq}"))
        });
        let row = if selected {
            row.bg(colors.selection)
        } else {
            row
        };
        row.relative()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(
                    move |view, event: &MouseDownEvent, window: &mut Window, cx| {
                        window.focus(&view.focus, cx);
                        view.click_line(seq, event.modifiers.shift, cx);
                    },
                ),
            )
            .on_mouse_move(cx.listener(move |view, event: &MouseMoveEvent, _, cx| {
                if event.dragging() {
                    view.drag_to_line(seq, cx);
                }
            }))
            .children(gutter)
    }
}
