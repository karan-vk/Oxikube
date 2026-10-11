//! Drawing the node panel: the frame (top row, allocation) and the helpers the sections share.
//! Everything drawn is ready-made in the panel's state: a frame builds elements, it computes
//! nothing from the cluster. The static sections are in `sections`, the two lists in `lists`.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use oxikube_domain::{AllocationLine, NodeAllocation};
use oxikube_ui::button::Button;
use oxikube_ui::layout::{Disableable as _, StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, Tokens, u};

use super::bars::allocation_bar;
use super::sections::{conditions, taints};
use super::{Fetch, NodePanel};
use crate::table::ToneColors;

/// The tallest the panel grows before it scrolls, before the UI zoom.
const MAX_HEIGHT: f32 = 520.;

pub(super) fn heading(tokens: &Tokens, id: &'static str, text: String) -> impl IntoElement {
    div()
        .debug_selector(move || format!("node-section-{id}"))
        .pt(u(tokens.spacing.lg))
        .pb(u(tokens.spacing.sm))
        .text_size(u(tokens.font.small))
        .font_semibold()
        .text_color(tokens.colors.text_muted)
        .child(text.to_uppercase())
}

pub(super) fn muted(tokens: &Tokens, text: impl Into<gpui::SharedString>) -> Div {
    div()
        .text_size(u(tokens.font.small))
        .text_color(tokens.colors.text_muted)
        .child(text.into())
}

pub(super) fn chip(tokens: &Tokens, color: gpui::Hsla, text: impl Into<gpui::SharedString>) -> Div {
    div()
        .px(u(tokens.spacing.md))
        .py(u(px(1.)))
        .rounded(u(tokens.radius.sm))
        .border_1()
        .border_color(color)
        .text_color(color)
        .text_size(u(tokens.font.small))
        .child(text.into())
}

impl NodePanel {
    pub(super) fn body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let tones = ToneColors::current(cx);
        let root = v_flex()
            .id("node-panel")
            .debug_selector(|| "node-panel".to_owned())
            .w_full()
            .max_h(u(px(MAX_HEIGHT)))
            .overflow_y_scroll()
            .px(u(tokens.spacing.lg))
            .pb(u(tokens.spacing.md))
            .border_b_1()
            .border_color(tokens.colors.border_variant);
        let Some(node) = &self.node else {
            return root
                .child(muted(&tokens, "Reading the node..."))
                .into_any_element();
        };
        let summary = &node.summary;
        let allocation = self
            .pods
            .as_ref()
            .map(|pods| NodeAllocation::new(summary, &pods.demand));
        let pods_note = self.pods_note();
        // What changes (allocation, taints, pods) first; the static lists (capacity, images, system
        // info) below, where the panel scrolls.
        root.child(self.top_row(&tokens, &tones, cx))
            .child(conditions(&tokens, &tones, &summary.conditions))
            .child(heading(&tokens, "allocation", "Allocation".to_owned()))
            .child(self.allocation(
                &tokens,
                &tones,
                allocation.as_ref().map(|a| a.lines()),
                pods_note,
            ))
            .child(heading(
                &tokens,
                "taints",
                format!("Taints ({})", summary.taints.len()),
            ))
            .child(taints(&tokens, &tones, &summary.taints))
            .child(heading(
                &tokens,
                "pods",
                format!("Pods ({})", self.pods.as_ref().map_or(0, |p| p.rows.len())),
            ))
            .child(self.pods_list(&tokens, &tones, cx))
            .child(heading(&tokens, "capacity", "Capacity".to_owned()))
            .child(self.capacity(&tokens))
            .child(heading(
                &tokens,
                "images",
                format!("Images ({})", node.detail.images.len()),
            ))
            .child(self.images_list(&tokens, cx))
            .child(heading(&tokens, "system", "System".to_owned()))
            .child(self.system(&tokens))
            .into_any_element()
    }

    /// Why the pods are not shown (yet), if they are not.
    pub(super) fn pods_note(&self) -> Option<String> {
        if self.pods.is_some() {
            return None;
        }
        Some(match &self.fetch {
            Fetch::Failed(why) => format!("The pods on this node could not be listed: {why}"),
            _ => "Reading the pods on this node...".to_owned(),
        })
    }

    /// The cordon badge and the button.
    fn top_row(&self, tokens: &Tokens, tones: &ToneColors, cx: &mut Context<Self>) -> AnyElement {
        let Some(node) = &self.node else {
            return div().into_any_element();
        };
        let cordoned = !node.summary.schedulable;
        let button = if cordoned {
            (
                "node-panel-uncordon",
                "Uncordon",
                "Allow new pods on this node again",
            )
        } else {
            (
                "node-panel-cordon",
                "Cordon",
                "Stop new pods from landing here; the pods already on it stay",
            )
        };
        let tooltip = self.blocked.clone().unwrap_or_else(|| button.2.to_owned());
        h_flex()
            .pt(u(tokens.spacing.md))
            .gap(u(tokens.spacing.md))
            .items_center()
            .when(cordoned, |row| {
                row.child(
                    div()
                        .debug_selector(|| "node-panel-cordoned".to_owned())
                        .child(chip(tokens, tones.warn, "Cordoned")),
                )
            })
            .child(div().flex_1())
            .child(
                div().debug_selector(move || button.0.to_owned()).child(
                    Button::new(button.0)
                        .xsmall()
                        .outline()
                        .label(button.1)
                        .tooltip(tooltip)
                        .disabled(self.blocked.is_some())
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_cordon(cx))),
                ),
            )
            .into_any_element()
    }

    fn allocation(
        &self,
        tokens: &Tokens,
        tones: &ToneColors,
        lines: Option<[&AllocationLine; 3]>,
        note: Option<String>,
    ) -> AnyElement {
        let Some(lines) = lines else {
            return div()
                .debug_selector(|| "node-allocation-pending".to_owned())
                .child(muted(tokens, note.unwrap_or_default()))
                .into_any_element();
        };
        let ids = [
            "node-allocation-cpu",
            "node-allocation-memory",
            "node-allocation-pods",
        ];
        v_flex()
            .gap(u(tokens.spacing.md))
            .children(lines.into_iter().zip(ids).map(|(line, id)| {
                // `980m of 3920m (25%)`, the bar, then what the limits add up to.
                let used = format!("{} of {}", line.requests_text(), line.allocatable_text());
                let limits = line
                    .limits
                    .map(|_| format!("limits {}", line.limits_text()));
                v_flex()
                    .debug_selector(move || id.to_owned())
                    .gap(u(px(2.)))
                    .child(
                        h_flex()
                            .justify_between()
                            .gap(u(tokens.spacing.md))
                            .child(div().font_semibold().child(line.resource.title()))
                            .child(muted(tokens, used)),
                    )
                    .child(allocation_bar(tokens, tones, line, id))
                    .children(limits.map(|text| {
                        muted(tokens, text).debug_selector(move || format!("{id}-limits"))
                    }))
            }))
            .into_any_element()
    }
}
