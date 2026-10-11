//! The panel's static sections: condition chips, taints, capacity beside allocatable, system
//! info.

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, div, px,
};
use oxikube_app::columns::Tone;
use oxikube_domain::NodeTaint;
use oxikube_domain::view::{Condition, ConditionStatus};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{Tokens, u};

use super::NodePanel;
use super::render::{chip, muted};
use crate::table::ToneColors;

impl NodePanel {
    pub(super) fn capacity(&self, tokens: &Tokens) -> AnyElement {
        let Some(node) = &self.node else {
            return div().into_any_element();
        };
        let cell = |text: String| div().flex_1().min_w_0().truncate().child(text);
        v_flex()
            .debug_selector(|| "node-capacity".to_owned())
            .child(
                h_flex()
                    .text_color(tokens.colors.text_muted)
                    .text_size(u(tokens.font.small))
                    .child(cell(String::new()))
                    .child(cell("Capacity".to_owned()))
                    .child(cell("Allocatable".to_owned())),
            )
            .children(node.detail.capacity.iter().map(|row| {
                h_flex()
                    .child(cell(row.name.to_string()).text_color(tokens.colors.text_muted))
                    .child(cell(row.capacity.as_deref().unwrap_or("-").to_owned()))
                    .child(cell(row.allocatable.as_deref().unwrap_or("-").to_owned()))
            }))
            .into_any_element()
    }

    pub(super) fn system(&self, tokens: &Tokens) -> AnyElement {
        let Some(node) = &self.node else {
            return div().into_any_element();
        };
        let detail = &node.detail;
        let rows = detail
            .system
            .iter()
            .map(|(label, value)| ((*label).to_owned(), value.to_string()))
            .chain(
                detail
                    .addresses
                    .iter()
                    .map(|(kind, address)| (kind.to_string(), address.to_string())),
            )
            .chain(
                detail
                    .pod_cidr
                    .iter()
                    .map(|c| ("Pod CIDR".to_owned(), c.to_string())),
            )
            .chain(
                detail
                    .provider_id
                    .iter()
                    .map(|p| ("Provider ID".to_owned(), p.to_string())),
            );
        v_flex()
            .debug_selector(|| "node-system".to_owned())
            .children(rows.map(|(label, value)| {
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .child(
                        div()
                            .flex_none()
                            .w(u(px(120.)))
                            .text_color(tokens.colors.text_muted)
                            .child(label),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(value))
            }))
            .into_any_element()
    }
}

pub(super) fn conditions(
    tokens: &Tokens,
    tones: &ToneColors,
    conditions: &[Condition],
) -> AnyElement {
    if conditions.is_empty() {
        return div().into_any_element();
    }
    h_flex()
        .debug_selector(|| "node-conditions".to_owned())
        .flex_wrap()
        .gap(u(tokens.spacing.sm))
        .pt(u(tokens.spacing.md))
        .children(conditions.iter().map(|c| {
            // `Ready` is good when true; every other condition (pressure, unavailable) is a
            // problem when true.
            let good = (&*c.kind == "Ready") == (c.status == ConditionStatus::True);
            let tone = match (c.status, good) {
                (ConditionStatus::Unknown, _) => Tone::Warn,
                (_, true) => Tone::Ok,
                (_, false) => Tone::Error,
            };
            let kind = c.kind.to_string();
            div()
                .debug_selector(move || format!("node-condition-{kind}"))
                .child(chip(
                    tokens,
                    tones.of(tone),
                    format!("{}: {}", c.kind, c.status.as_str()),
                ))
        }))
        .into_any_element()
}

pub(super) fn taints(tokens: &Tokens, tones: &ToneColors, taints: &[NodeTaint]) -> AnyElement {
    if taints.is_empty() {
        return muted(tokens, "No taints.")
            .debug_selector(|| "node-no-taints".to_owned())
            .into_any_element();
    }
    h_flex()
        .debug_selector(|| "node-taints".to_owned())
        .flex_wrap()
        .gap(u(tokens.spacing.sm))
        .children(taints.iter().enumerate().map(|(ix, taint)| {
            let color = if taint.is_unschedulable() {
                tones.warn
            } else {
                tones.neutral
            };
            div()
                .debug_selector(move || format!("node-taint-{ix}"))
                .child(chip(tokens, color, taint.to_string()))
        }))
        .into_any_element()
}
