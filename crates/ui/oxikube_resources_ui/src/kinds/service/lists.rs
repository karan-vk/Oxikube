//! The Service panel's two lists: the EndpointSlices and the backing pods (each row opens its
//! object's detail). Both are capped by the model.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _, Stateful,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use oxikube_app::columns::Tone;
use oxikube_domain::ids::ResourceRef;
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
        const TITLE: &str = "Endpoint slices";
        match &view.slices {
            Slices::Loading => noted(tokens, TITLE, "Loading…"),
            Slices::Failed(why) => noted(
                tokens,
                TITLE,
                &format!("Endpoint slices cannot be read: {why}"),
            ),
            Slices::Listed { rows, .. } if rows.is_empty() => noted(
                tokens,
                TITLE,
                "No endpoint slices: nothing serves this service.",
            ),
            Slices::Listed { rows, more, .. } => section(tokens, TITLE, rows.len() + more)
                .children(
                    rows.iter()
                        .enumerate()
                        .map(|(i, row)| self.slice_row(row, i, tokens, tones, cx)),
                )
                .children((*more > 0).then(|| note(tokens, &format!("and {more} more"))))
                .into_any_element(),
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
        let tone = if row.counts.not_ready == 0 && row.counts.ready > 0 {
            Tone::Ok
        } else if row.counts.ready == 0 {
            Tone::Error
        } else {
            Tone::Warn
        };
        link_row("service-slice", index, row.target.clone(), tokens, cx)
            .child(name_cell(&row.name))
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
        const TITLE: &str = "Backing pods";
        match &view.pods {
            Pods::None(reason) => noted_as("service-no-pods", tokens, TITLE, reason.explanation()),
            Pods::Loading => noted(tokens, TITLE, "Loading…"),
            Pods::Failed(why) => noted(tokens, TITLE, &format!("Pods cannot be read: {why}")),
            Pods::Listed { rows, .. } if rows.is_empty() => noted_as(
                "service-pods-empty",
                tokens,
                TITLE,
                "No pod matches the selector.",
            ),
            Pods::Listed { rows, more, total } => section(tokens, TITLE, *total)
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
        link_row("service-pod", index, row.target.clone(), tokens, cx)
            .child(name_cell(&row.name))
            .child(cell(row.ready.clone(), 36., colors.text_muted))
            .child(cell(row.status.clone(), 110., tones.of(row.tone)))
            .child(cell(row.ip.clone(), 100., colors.text_muted))
            .into_any_element()
    }
}

/// A titled block holding one note.
fn noted(tokens: &Tokens, title: &'static str, text: &str) -> AnyElement {
    section(tokens, title, 0)
        .child(note(tokens, text))
        .into_any_element()
}

/// [`noted`], its note findable by `selector` in tests.
fn noted_as(
    selector: &'static str,
    tokens: &Tokens,
    title: &'static str,
    text: &str,
) -> AnyElement {
    section(tokens, title, 0)
        .child(
            div()
                .debug_selector(move || selector.to_owned())
                .child(note(tokens, text)),
        )
        .into_any_element()
}

/// A clickable row that opens `target`'s detail; `id-index` names it for tests.
fn link_row(
    id: &'static str,
    index: usize,
    target: ResourceRef,
    tokens: &Tokens,
    cx: &mut Context<ServicePanel>,
) -> Stateful<gpui::Div> {
    let hover = tokens.colors.element_hover;
    h_flex()
        .id((id, index))
        .debug_selector(move || format!("{id}-{index}"))
        .items_center()
        .gap(u(tokens.spacing.md))
        .px(u(tokens.spacing.lg))
        .py(u(px(1.)))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .on_click(cx.listener(move |this, _, _, cx| this.open_object(target.clone(), cx)))
}

/// The flexible first cell of a row: an object's name.
fn name_cell(name: &str) -> gpui::Div {
    div().flex_1().min_w_0().truncate().child(name.to_owned())
}
