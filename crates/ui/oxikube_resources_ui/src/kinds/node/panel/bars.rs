//! The allocation bar: requests as the fill, limits as a mark.

use gpui::{
    Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, div, px, relative,
};
use oxikube_domain::AllocationLine;
use oxikube_ui::{Tokens, u};

use crate::table::ToneColors;

/// Requests past this share of allocatable turn the bar amber, past [`FULL`] red.
const BUSY: f32 = 0.7;
/// See [`BUSY`].
const FULL: f32 = 0.9;

/// The bar's fill colour for a line.
pub(super) fn fill_color(line: &AllocationLine, tones: &ToneColors) -> Hsla {
    if line.is_overcommitted() || line.request_fill() >= FULL {
        tones.error
    } else if line.request_fill() >= BUSY {
        tones.warn
    } else {
        tones.ok
    }
}

/// The bar of `line`: a track, the requests fill and, when the line has limits, a thin mark where
/// they reach (pinned at the end when they exceed allocatable). `id` names the pieces for tests:
/// `{id}-bar` for the track and `{id}-fill` for the fill.
pub(super) fn allocation_bar(
    tokens: &Tokens,
    tones: &ToneColors,
    line: &AllocationLine,
    id: &'static str,
) -> impl IntoElement {
    let fill = line.request_fill();
    let mark = line
        .limits
        .filter(|limits| *limits > 0 && line.allocatable.is_some())
        .map(|_| line.limit_fill());
    div()
        .debug_selector(move || format!("{id}-bar"))
        .relative()
        .h(u(px(8.)))
        .w_full()
        .rounded(u(tokens.radius.sm))
        .bg(tokens.colors.element)
        .overflow_hidden()
        .child(
            div()
                .debug_selector(move || format!("{id}-fill"))
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(relative(fill))
                .bg(fill_color(line, tones)),
        )
        .children(mark.map(|at| {
            div()
                .absolute()
                .top_0()
                .h_full()
                .w(u(px(2.)))
                .left(relative((at - 0.01).max(0.0)))
                .bg(tokens.colors.text_muted)
        }))
}
