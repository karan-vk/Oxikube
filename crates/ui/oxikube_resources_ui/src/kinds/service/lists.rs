//! The Service panel's two lists: the EndpointSlices and the backing pods (each row opens its
//! object's detail). Both are capped by the model.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use oxikube_app::columns::Tone;
use oxikube_ui::layout::h_flex;
use oxikube_ui::{Tokens, u};

use super::model::{PodRow, Pods, ServiceView, SliceRow, Slices};
use super::panel::ServicePanel;
use crate::kinds::parts::{cell, note, section};
use crate::table::ToneColors;

impl ServicePanel {
    pub(super) fn slices(
        &self,
        view: &ServiceView,
        tokens: &Tokens,
        tones: &ToneColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match &view.slices {
            Slices::Loading => section(tokens, "Endpoint slices", 0)
                .child(note(tokens, "Loading…"))
                .into_any_element(),
            Slices::Failed(why) => section(tokens, "Endpoint slices", 0)
                .child(note(
                    tokens,
                    &format!("Endpoint slices cannot be read: {why}"),
                ))
                .into_any_element(),
            Slices::Listed { rows, more, .. } if rows.is_empty() => {
                section(tokens, "Endpoint slices", 0)
                    .child(note(
                        tokens,
                        "No endpoint slices: nothing serves this service.",
                    ))
                    .into_any_element()
            }
            Slices::Listed { rows, more, .. } => {
                section(tokens, "Endpoint slices", rows.len() + more)
                    .children(
                        rows.iter()
                            .enumerate()
                            .map(|(i, row)| self.slice_row(row, i, tokens, tones, cx)),
                    )
                    .children((*more > 0).then(|| note(tokens, &format!("and {more} more"))))
                    .into_any_element()
            }
        }
    }

    fn slice_row(
        &self,
        row: &SliceRow,
        index: usize,
        tokens: &Tokens,
        tones: &ToneColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = tokens.colors;
        let target = row.target.clone();
        let tone = if row.counts.not_ready == 0 && row.counts.ready > 0 {
            Tone::Ok
        } else if row.counts.ready == 0 {
            Tone::Error
        } else {
            Tone::Warn
        };
        h_flex()
            .id(("service-slice", index))
            .debug_selector(move || format!("service-slice-{index}"))
            .items_center()
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.lg))
            .py(u(px(1.)))
            .cursor_pointer()
            .hover(|style| style.bg(colors.element_hover))
            .on_click(cx.listener(move |this, _, _, cx| this.open_object(target.clone(), cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(row.name.to_string()),
            )
            .child(cell(row.address_type.clone(), 56., colors.text_muted))
            .child(cell(row.ports.clone(), 90., colors.text_muted))
            .child(
                div()
                    .flex_none()
                    .w(u(px(150.)))
                    .truncate()
                    .text_right()
                    .text_color(tones.of(tone))
                    .child(row.counts.summary()),
            )
            .into_any_element()
    }

    pub(super) fn pods(
        &self,
        view: &ServiceView,
        tokens: &Tokens,
        tones: &ToneColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match &view.pods {
            Pods::None(reason) => section(tokens, "Backing pods", 0)
                .child(
                    div()
                        .debug_selector(|| "service-no-pods".to_owned())
                        .child(note(tokens, reason.explanation())),
                )
                .into_any_element(),
            Pods::Loading => section(tokens, "Backing pods", 0)
                .child(note(tokens, "Loading…"))
                .into_any_element(),
            Pods::Failed(why) => section(tokens, "Backing pods", 0)
                .child(note(tokens, &format!("Pods cannot be read: {why}")))
                .into_any_element(),
            Pods::Listed { rows, .. } if rows.is_empty() => section(tokens, "Backing pods", 0)
                .child(
                    div()
                        .debug_selector(|| "service-pods-empty".to_owned())
                        .child(note(tokens, "No pod matches the selector.")),
                )
                .into_any_element(),
            Pods::Listed { rows, more, total } => section(tokens, "Backing pods", *total)
                .children(
                    rows.iter()
                        .enumerate()
                        .map(|(i, row)| self.pod_row(row, i, tokens, tones, cx)),
                )
                .children((*more > 0).then(|| note(tokens, &format!("and {more} more"))))
                .into_any_element(),
        }
    }

    fn pod_row(
        &self,
        row: &PodRow,
        index: usize,
        tokens: &Tokens,
        tones: &ToneColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = tokens.colors;
        let target = row.target.clone();
        h_flex()
            .id(("service-pod", index))
            .debug_selector(move || format!("service-pod-{index}"))
            .items_center()
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.lg))
            .py(u(px(1.)))
            .cursor_pointer()
            .hover(|style| style.bg(colors.element_hover))
            .on_click(cx.listener(move |this, _, _, cx| this.open_object(target.clone(), cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(row.name.to_string()),
            )
            .child(cell(row.ready.clone(), 36., colors.text_muted))
            .child(cell(row.status.clone(), 110., tones.of(row.tone)))
            .child(cell(row.ip.clone(), 100., colors.text_muted))
            .into_any_element()
    }
}
