//! The Events tab: the events about the object, newest first, in a virtualised list. Warnings
//! stand out; a repeated event shows its count.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    div, list, px,
};
use oxikube_domain::Age;
use oxikube_domain::event::EventType;
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, u};

use super::parts::{empty, full_width, skeleton};
use super::view::DetailView;
use crate::table::ToneColors;

impl DetailView {
    /// The Events tab: a skeleton until the feed answers, a note when there are none or the feed
    /// is unavailable, else the list.
    pub(super) fn events_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let note = |selector: &'static str, text: String| {
            div()
                .debug_selector(move || selector.to_owned())
                .p(u(tokens.spacing.xl))
                .text_color(tokens.colors.text_muted)
                .child(text)
                .into_any_element()
        };
        if let Some(error) = &self.events.error {
            return note(
                "detail-events-error",
                format!("Events are not available: {error}"),
            );
        }
        if !self.events.ready {
            return skeleton(&tokens, "detail-events-loading", &[300., 240., 280.]);
        }
        if self.events.rows.is_empty() {
            return note(
                "detail-events-empty",
                "No events for this object.".to_owned(),
            );
        }
        div()
            .debug_selector(|| "detail-events".to_owned())
            .size_full()
            .child(
                list(
                    self.events_list.clone(),
                    cx.processor(|this, ix, _, cx| full_width(this.event_row(ix, cx))),
                )
                .size_full(),
            )
            .into_any_element()
    }

    /// One event. Only rows on screen are built.
    fn event_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(event) = self.events.rows.get(ix) else {
            return empty();
        };
        let now = self.now();
        let tokens = cx.tokens();
        let tones = ToneColors::current(cx);
        let color = match event.kind {
            EventType::Warning => tones.warn,
            EventType::Normal | EventType::Other => tones.neutral,
        };
        let age = event
            .last_seen
            .map(|at| Age::between(at, now).to_kubectl_string())
            .unwrap_or_default();
        let source = event.source.as_deref().unwrap_or("").to_owned();
        v_flex()
            .debug_selector(move || format!("detail-event-{ix}"))
            .gap(u(px(1.)))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .child(
                        div()
                            .font_semibold()
                            .text_color(color)
                            .child(event.reason.to_string()),
                    )
                    .when(event.count > 1, |row| {
                        row.child(div().child(format!("x{}", event.count)))
                    })
                    .child(
                        div()
                            .flex_1()
                            .text_color(tokens.colors.text_muted)
                            .text_size(u(tokens.font.small))
                            .child(source),
                    )
                    .child(
                        div()
                            .text_color(tokens.colors.text_muted)
                            .text_size(u(tokens.font.small))
                            .child(age),
                    ),
            )
            .child(div().whitespace_normal().child(event.message.clone()))
            .into_any_element()
    }
}
