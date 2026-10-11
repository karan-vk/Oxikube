//! The pods and images lists: virtualised `uniform_list`s in a fixed-height box, so a node with
//! a hundred pods or two hundred images costs the rows on screen.

use std::ops::Range;

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px, uniform_list,
};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, Tokens, u};

use super::NodePanel;
use super::data::{PodRow, PodsOnNode};
use super::render::muted;
use crate::table::ToneColors;

/// A pod or image row, before the UI zoom.
const ROW_HEIGHT: f32 = 22.;
/// Rows a list shows before it scrolls.
const LIST_ROWS: f32 = 5.;

impl NodePanel {
    pub(super) fn pods_list(
        &self,
        tokens: &Tokens,
        tones: &ToneColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(pods) = &self.pods else {
            return muted(tokens, self.pods_note().unwrap_or_default()).into_any_element();
        };
        if pods.rows.is_empty() {
            return muted(tokens, "No pods run on this node.")
                .debug_selector(|| "node-pods-empty".to_owned())
                .into_any_element();
        }
        let tones = *tones;
        let count = pods.rows.len();
        div()
            .debug_selector(|| "node-pods".to_owned())
            .h(u(px(ROW_HEIGHT * (count as f32).min(LIST_ROWS))))
            .child(
                uniform_list(
                    "node-pod-rows",
                    count,
                    cx.processor(move |this, range: Range<usize>, _, cx| {
                        this.pod_rows(range, &tones, cx)
                    }),
                )
                .track_scroll(&self.pods_scroll)
                .size_full(),
            )
            .into_any_element()
    }

    fn pod_rows(
        &self,
        range: Range<usize>,
        tones: &ToneColors,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let tokens = cx.tokens();
        let Some(PodsOnNode { rows, .. }) = &self.pods else {
            return Vec::new();
        };
        range
            .filter_map(|ix| Some((ix, rows.get(ix)?)))
            .map(|(ix, row)| pod_row(&tokens, tones, ix, row, cx))
            .collect()
    }

    pub(super) fn images_list(&self, tokens: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let Some(node) = &self.node else {
            return div().into_any_element();
        };
        let count = node.detail.images.len();
        if count == 0 {
            return muted(tokens, "The node reports no images.")
                .debug_selector(|| "node-images-empty".to_owned())
                .into_any_element();
        }
        div()
            .debug_selector(|| "node-images".to_owned())
            .h(u(px(ROW_HEIGHT * (count as f32).min(LIST_ROWS))))
            .child(
                uniform_list(
                    "node-image-rows",
                    count,
                    cx.processor(|this, range: Range<usize>, _, cx| this.image_rows(range, cx)),
                )
                .track_scroll(&self.images_scroll)
                .size_full(),
            )
            .into_any_element()
    }

    fn image_rows(&self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let tokens = cx.tokens();
        let Some(node) = &self.node else {
            return Vec::new();
        };
        range
            .filter_map(|ix| Some((ix, node.detail.images.get(ix)?)))
            .map(|(ix, image)| {
                h_flex()
                    .debug_selector(move || format!("node-image-{ix}"))
                    .w_full()
                    .h(u(px(ROW_HEIGHT)))
                    .gap(u(tokens.spacing.md))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(image.name.to_string()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(tokens.colors.text_muted)
                            .text_size(u(tokens.font.small))
                            .child(image.size_text()),
                    )
                    .into_any_element()
            })
            .collect()
    }
}

fn pod_row(
    tokens: &Tokens,
    tones: &ToneColors,
    ix: usize,
    row: &PodRow,
    cx: &mut Context<NodePanel>,
) -> AnyElement {
    let label = format!("{}/{}", row.namespace, row.name);
    let open = row.clone();
    h_flex()
        .id(("node-pod", ix))
        .debug_selector(move || format!("node-pod-{ix}"))
        .w_full()
        .h(u(px(ROW_HEIGHT)))
        .gap(u(tokens.spacing.md))
        .items_center()
        .cursor_pointer()
        .hover(|row| row.bg(tokens.colors.element_hover))
        .on_click(cx.listener(move |this, _, _, cx| this.open_pod(&open, cx)))
        .child(div().flex_1().min_w_0().truncate().child(label))
        .child(
            div()
                .flex_none()
                .text_color(tones.of(row.tone))
                .child(row.status.to_string()),
        )
        .child(
            div()
                .flex_none()
                .w(u(px(110.)))
                .text_right()
                .text_color(tokens.colors.text_muted)
                .text_size(u(tokens.font.small))
                .child(format!("{} · {}", row.cpu, row.memory)),
        )
        .into_any_element()
}

impl NodePanel {
    /// Opens a pod of the list: sends `resource::Open`.
    fn open_pod(&mut self, row: &PodRow, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let (cluster, dispatcher) = {
            let view = detail.read(cx);
            (
                view.target().cluster.clone(),
                view.deps().dispatcher.clone(),
            )
        };
        dispatcher.dispatch(
            oxikube_domain::Command::ResourceOpen {
                target: row.target(&cluster),
            },
            cx,
        );
    }
}
