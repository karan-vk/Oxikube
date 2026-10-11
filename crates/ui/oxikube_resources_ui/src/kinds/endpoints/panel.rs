//! [`EndpointsPanel`]: the panel of an Endpoints or EndpointSlice detail. It reads the object the
//! detail holds and starts no feed of its own.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AnyView, App, AppContext as _, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, WeakEntity, Window,
    div, px,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Tokens, u};

use super::model::{EndpointRow, EndpointsView};
use crate::detail::DetailView;
use crate::kinds::parts::{cell, note, section};
use crate::table::ToneColors;

/// The panel of one Endpoints or EndpointSlice detail.
pub struct EndpointsPanel {
    detail: WeakEntity<DetailView>,
    view: Option<EndpointsView>,
    _follow: Option<gpui::Subscription>,
}

/// The factory the three kinds' specs register.
pub(super) fn build(detail: WeakEntity<DetailView>, _: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| EndpointsPanel::new(detail, cx)).into()
}

impl EndpointsPanel {
    fn new(detail: WeakEntity<DetailView>, cx: &mut Context<Self>) -> Self {
        let follow = detail
            .upgrade()
            .map(|view| cx.observe(&view, |this, _, cx| this.sync(cx)));
        let this = Self {
            detail,
            view: None,
            _follow: follow,
        };
        // The detail is being drawn (it builds its panel): read it once it is done.
        let weak = cx.weak_entity();
        cx.defer(move |cx| {
            weak.update(cx, |this, cx| this.sync(cx)).ok();
        });
        this
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let view = {
            let detail = detail.read(cx);
            let cluster = detail.target().cluster.clone();
            detail
                .resource()
                .and_then(|resource| EndpointsView::of(resource, &cluster))
        };
        if view.is_some() && view != self.view {
            self.view = view;
            cx.notify();
        }
    }

    /// Opens the detail of `target` (the Service, or a pod): sends `resource::Open`.
    fn open(&mut self, target: ResourceRef, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let dispatcher = detail.read(cx).deps().dispatcher.clone();
        dispatcher.dispatch(Command::ResourceOpen { target }, cx);
    }

    #[cfg(test)]
    pub(super) fn view(&self) -> Option<&EndpointsView> {
        self.view.as_ref()
    }
}

impl Render for EndpointsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let tones = ToneColors::current(cx);
        let root = v_flex()
            .id("endpoints-panel")
            .debug_selector(|| "endpoints-panel".to_owned())
            .w_full()
            .max_h(u(px(300.)))
            .overflow_y_scroll()
            .border_b_1()
            .border_color(colors.border_variant)
            .pb(u(tokens.spacing.md));
        let Some(view) = self.view.as_ref() else {
            return root.child(note(&tokens, "Reading the endpoints…"));
        };
        let service = view.service.clone();
        let mut facts = vec![("Ready", view.counts.summary())];
        if !view.address_type.is_empty() {
            facts.push(("Address type", view.address_type.clone()));
        }
        if !view.ports.is_empty() {
            facts.push(("Ports", view.ports.clone()));
        }
        root.child(
            v_flex()
                .px(u(tokens.spacing.lg))
                .pt(u(tokens.spacing.lg))
                .children(facts.into_iter().enumerate().map(|(i, (label, value))| {
                    h_flex()
                        .debug_selector(move || format!("endpoints-fact-{i}"))
                        .gap(u(tokens.spacing.md))
                        .child(cell(label, 110., colors.text_muted))
                        .child(div().flex_1().min_w_0().truncate().child(value))
                }))
                .when_some(service, |col, service| {
                    col.child(
                        h_flex()
                            .id("endpoints-service")
                            .debug_selector(|| "endpoints-service".to_owned())
                            .gap(u(tokens.spacing.md))
                            .cursor_pointer()
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.open(service.clone(), cx)),
                            )
                            .child(cell("Service", 110., colors.text_muted))
                            .child(div().text_color(colors.accent).child("open the service")),
                    )
                }),
        )
        .child(
            section(&tokens, "Endpoints", view.counts.total() as usize)
                .children(
                    view.rows
                        .iter()
                        .enumerate()
                        .map(|(i, row)| self.row(row, i, &tokens, &tones, cx)),
                )
                .children(
                    (view.more > 0).then(|| note(&tokens, &format!("and {} more", view.more))),
                ),
        )
    }
}

impl EndpointsPanel {
    fn row(
        &self,
        row: &EndpointRow,
        index: usize,
        tokens: &Tokens,
        tones: &ToneColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = tokens.colors;
        let target = row.target.clone();
        h_flex()
            .id(("endpoint", index))
            .debug_selector(move || format!("endpoint-{index}"))
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.lg))
            .py(u(px(1.)))
            .when(target.is_some(), |row| {
                row.cursor_pointer()
                    .hover(|style| style.bg(colors.element_hover))
            })
            .when_some(target.clone(), |row, (_, reference)| {
                row.on_click(cx.listener(move |this, _, _, cx| this.open(reference.clone(), cx)))
            })
            .child(cell(row.addresses.clone(), 150., colors.text))
            .child(cell(row.state, 90., tones.of(row.tone)))
            .child(cell(row.node.clone(), 120., colors.text_muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(colors.text_muted)
                    .child(
                        target
                            .map(|(name, _)| format!("pod/{name}"))
                            .unwrap_or_default(),
                    ),
            )
            .into_any_element()
    }
}
