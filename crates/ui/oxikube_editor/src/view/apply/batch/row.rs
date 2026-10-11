//! The row both the preview dialog and the results panel list a document with: its number, a
//! coloured word for what the dry run or the apply said, its label and a line of detail.

use gpui::{Hsla, InteractiveElement as _, ParentElement as _, Stateful, Styled as _, div, px};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{Tokens, u};

/// A document's row `ix` of the list named `prefix` (its element id and `prefix-{ix}` debug
/// selector; the word also has `prefix-{ix}-{word}`). `chip` is the word and its colour. The caller adds the click and any highlight.
pub(super) fn document_row(
    prefix: &'static str,
    ix: usize,
    height: f32,
    chip: (&'static str, Hsla),
    label: &str,
    detail: String,
    tokens: &Tokens,
) -> Stateful<gpui::Div> {
    let (word, tone) = chip;
    h_flex()
        .id((prefix, ix))
        .debug_selector(move || format!("{prefix}-{ix}"))
        .h(u(px(height)))
        .w_full()
        .px(u(tokens.spacing.md))
        .gap(u(tokens.spacing.md))
        .items_center()
        .text_size(u(tokens.font.small))
        .child(
            div()
                .flex_none()
                .w(u(px(18.)))
                .text_color(tokens.colors.text_muted)
                .child((ix + 1).to_string()),
        )
        .child(
            div()
                .debug_selector(move || format!("{prefix}-{ix}-{word}"))
                .flex_none()
                .w(u(px(78.)))
                .text_color(tone)
                .child(word),
        )
        .child(div().flex_none().child(label.to_owned()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(tokens.colors.text_muted)
                .child(detail),
        )
}
