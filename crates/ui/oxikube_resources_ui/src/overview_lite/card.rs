//! A tile's card: the number and caption (a click opens the kind's list), the stacked status bar
//! and the legend under it (a click on a segment or a legend entry opens the list filtered to that
//! status).
//!
//! The header and the bar are siblings, never nested, so a click is handled by exactly one of
//! them. The bar's proportions come from [`segments`], computed per frame from the counts the
//! view already holds (a handful of integers, no allocation beyond the segment list).

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
    px, relative,
};
use oxikube_app::CountState;
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::tile::TileTone;
use oxikube_ui::tooltip::Tooltip;
use oxikube_ui::{ActiveTokens as _, u};

use super::bars::{Segment, colour, hover, segments};
use super::face::face;
use super::tiles::Tile;
use super::view::WorkloadsOverview;

/// Width of a card in unscaled pixels.
const CARD_WIDTH: f32 = 216.;
/// Height of the bar in unscaled pixels.
const BAR_HEIGHT: f32 = 8.;
/// The narrowest a segment is drawn, so a single pod among thousands stays visible and clickable.
const MIN_SEGMENT: f32 = 4.;

impl WorkloadsOverview {
    /// The card of `tile`, whose counts are `state`.
    pub(super) fn card(
        &self,
        tile: &Tile,
        state: &CountState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let face = face(state);
        let id = tile.id.clone();
        let segs = state
            .count()
            .map(|count| segments(&tile.target.gvk.group, &tile.target.gvk.kind, &count))
            .unwrap_or_default();
        let value_colour = match face.tone {
            TileTone::Neutral => colors.text,
            TileTone::Good => colors.success,
            TileTone::Warn => colors.warning,
            TileTone::Muted => colors.text_muted,
        };
        let click_id = id.clone();
        let header = v_flex()
            .id(SharedString::from(format!("overview-tile-{id}")))
            .debug_selector({
                let selector = format!("overview-tile-{id}");
                move || selector
            })
            .gap(u(tokens.spacing.sm))
            .cursor_pointer()
            .rounded(u(tokens.radius.sm))
            .hover(move |style| style.bg(colors.element_hover))
            .on_click(cx.listener(move |this, _, _, cx| this.open_tile(&click_id, cx)))
            .child(
                div()
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .truncate()
                    .child(tile.title.clone()),
            )
            .child(
                div()
                    .text_size(u(px(26.)))
                    .text_color(value_colour)
                    .font_semibold()
                    .truncate()
                    .child(face.value),
            )
            .child(
                div()
                    .h(u(px(16.)))
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .truncate()
                    .child(face.caption),
            )
            .when_some(face.hover.map(SharedString::from), |header, text| {
                header.tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
            });
        v_flex()
            .w(u(px(CARD_WIDTH)))
            .flex_none()
            .gap(u(tokens.spacing.md))
            .p(u(tokens.spacing.lg))
            .rounded(u(tokens.radius.md))
            .border_1()
            .border_color(colors.border_variant)
            .bg(colors.surface)
            .child(header)
            .child(self.bar(tile, &segs, cx))
            .child(self.legend(tile, &segs, cx))
            .into_any_element()
    }

    /// The stacked bar: one clickable segment per non-empty bucket, as wide as its share.
    fn bar(&self, tile: &Tile, segs: &[Segment], cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let track = h_flex()
            .h(u(px(BAR_HEIGHT)))
            .w_full()
            .gap(u(px(1.)))
            .rounded(u(tokens.radius.sm))
            .overflow_hidden()
            .bg(colors.element);
        track
            .children(segs.iter().map(|seg| {
                let (tile_id, bucket) = (tile.id.clone(), seg.bucket);
                let text = SharedString::from(hover(seg));
                div()
                    .id(SharedString::from(format!(
                        "overview-bar-{tile_id}-{}",
                        bucket.word()
                    )))
                    .debug_selector({
                        let selector = format!("overview-bar-{tile_id}-{}", bucket.word());
                        move || selector
                    })
                    .h_full()
                    .w(relative(seg.fraction))
                    .min_w(u(px(MIN_SEGMENT)))
                    .bg(colour(bucket, &colors))
                    .cursor_pointer()
                    .hover(|style| style.opacity(0.8))
                    .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_bucket(&tile_id, bucket, cx);
                    }))
            }))
            .into_any_element()
    }

    /// The legend: a dot, the bucket and its count, one per segment; each filters like its segment.
    fn legend(&self, tile: &Tile, segs: &[Segment], cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        h_flex()
            .flex_wrap()
            .gap_x(u(tokens.spacing.md))
            .gap_y(u(tokens.spacing.xs))
            .min_h(u(px(16.)))
            .children(segs.iter().map(|seg| {
                let (tile_id, bucket) = (tile.id.clone(), seg.bucket);
                h_flex()
                    .id(SharedString::from(format!(
                        "overview-legend-{tile_id}-{}",
                        bucket.word()
                    )))
                    .debug_selector({
                        let selector = format!("overview-legend-{tile_id}-{}", bucket.word());
                        move || selector
                    })
                    .gap(u(tokens.spacing.xs))
                    .items_center()
                    .cursor_pointer()
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .hover(move |style| style.text_color(colors.text))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_bucket(&tile_id, bucket, cx);
                    }))
                    .child(
                        div()
                            .size(u(px(8.)))
                            .flex_none()
                            .rounded_full()
                            .bg(colour(bucket, &colors)),
                    )
                    .child(format!("{} {}", bucket.label(), seg.count))
            }))
            .into_any_element()
    }
}
