//! What the ConfigMap and Secret detail panels share: the key list's geometry and its two
//! one-line states.

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, Styled as _,
};
use gpui::{div, px};
use oxikube_ui::layout::v_flex;
use oxikube_ui::{Tokens, u};

/// The height of a row, at 100 % zoom: uniform, as the virtualised list needs.
pub(super) const ROW_HEIGHT: f32 = 26.;

/// How many rows the list shows before it scrolls.
pub(super) const VISIBLE_ROWS: usize = 8;

/// The panel while its object is being read: `id` is the panel's element id and test selector.
pub(super) fn reading(tokens: &Tokens, id: &'static str, kind: &str) -> AnyElement {
    v_flex()
        .id(id)
        .debug_selector(move || id.to_owned())
        .p(u(tokens.spacing.lg))
        .text_color(tokens.colors.text_muted)
        .text_size(u(tokens.font.small))
        .child(format!("Reading the {kind}…"))
        .into_any_element()
}

/// The line that stands where the list would be, when the object has no keys.
pub(super) fn empty(tokens: &Tokens, selector: &'static str, text: &'static str) -> AnyElement {
    div()
        .debug_selector(move || selector.to_owned())
        .px(u(tokens.spacing.lg))
        .py(u(tokens.spacing.md))
        .text_color(tokens.colors.text_muted)
        .child(text)
        .into_any_element()
}

/// The height of a list showing `rows` rows, capped at [`VISIBLE_ROWS`].
pub(super) fn list_height(rows: usize) -> gpui::Pixels {
    u(px(ROW_HEIGHT * rows.min(VISIBLE_ROWS) as f32))
}
