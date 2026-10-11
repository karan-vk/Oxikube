//! The NetworkPolicy panel: the selected pods, the policy types and each direction's rules
//! (peers by pod selector, namespace selector or IP block with its exceptions; ports and
//! protocols), with default-deny and allow-all spelled out.

use gpui::prelude::*;
use gpui::{AnyElement, div};
use oxikube_app::columns::Tone;
use oxikube_domain::view::network::{NetworkPolicySummary, PolicyEffect, PolicyRule, PolicyType};
use oxikube_ui::layout::v_flex;
use oxikube_ui::u;

use super::panel::NetworkPanel;
use super::widgets::{Look, MAX_ROWS, chip, field, heading, more_rows, note, shell};

impl NetworkPanel {
    pub(super) fn policy(&self, look: &Look, summary: &NetworkPolicySummary) -> AnyElement {
        shell(look, "net-policy")
            .child(field(look, "net-policy-selector", "Applies to", {
                match summary.pod_selector_text().as_str() {
                    "all pods" => "every pod in this namespace".to_owned(),
                    labels => format!("pods {labels}"),
                }
            }))
            .child(field(
                look,
                "net-policy-types",
                "Policy types",
                summary.policy_types_text(),
            ))
            .child(direction(look, summary, PolicyType::Ingress))
            .child(direction(look, summary, PolicyType::Egress))
            .into_any_element()
    }
}

/// The element ids of one direction's blocks, and the word its peers go by.
struct Ids {
    section: &'static str,
    heading: &'static str,
    effect: &'static str,
    note: &'static str,
    rule: &'static str,
    peers: &'static str,
}

const INGRESS: Ids = Ids {
    section: "net-policy-ingress",
    heading: "net-policy-ingress-heading",
    effect: "net-policy-ingress-effect",
    note: "net-policy-ingress-note",
    rule: "net-policy-ingress-rule",
    peers: "From",
};

const EGRESS: Ids = Ids {
    section: "net-policy-egress",
    heading: "net-policy-egress-heading",
    effect: "net-policy-egress-effect",
    note: "net-policy-egress-note",
    rule: "net-policy-egress-rule",
    peers: "To",
};

/// One direction: its heading with the effect chip, the sentence about it, then its rules.
fn direction(look: &Look, summary: &NetworkPolicySummary, which: PolicyType) -> AnyElement {
    let (rules, ids, noun) = match which {
        PolicyType::Ingress => (&summary.ingress, &INGRESS, "ingress"),
        PolicyType::Egress => (&summary.egress, &EGRESS, "egress"),
    };
    let effect = summary.effect(which);
    let (label, tone, sentence) = match effect {
        PolicyEffect::NotCovered => (
            "Not covered",
            Tone::Neutral,
            format!("This policy does not cover {noun}; other policies decide it."),
        ),
        PolicyEffect::DenyAll => (
            "Deny all",
            Tone::Warn,
            format!(
                "Default deny: no rule allows {noun}, so all {noun} traffic of the selected pods is dropped."
            ),
        ),
        PolicyEffect::AllowAll => (
            "Allow all",
            Tone::Neutral,
            format!(
                "A rule with no peers and no ports allows all {noun} traffic of the selected pods."
            ),
        ),
        PolicyEffect::Restricted => (
            "Restricted",
            Tone::Ok,
            format!(
                "Only the {noun} traffic below is allowed for the selected pods; the rest is dropped."
            ),
        ),
    };
    v_flex()
        .debug_selector(move || ids.section.to_owned())
        .gap(u(look.tokens.spacing.sm))
        .child(heading(look, ids.heading, noun, Some(rules.len())))
        .child(chip(look, ids.effect, label, tone))
        .child(note(look, ids.note, sentence))
        .children(
            rules
                .iter()
                .take(MAX_ROWS)
                .enumerate()
                .map(|(i, rule)| rule_block(look, ids, i, rule)),
        )
        .children(more_rows(look, rules.len().saturating_sub(MAX_ROWS)))
        .into_any_element()
}

/// One rule: its peers, one per line, and its ports.
fn rule_block(look: &Look, ids: &Ids, index: usize, rule: &PolicyRule) -> AnyElement {
    let peers: Vec<String> = if rule.peers.is_empty() {
        vec![rule.peers_text()]
    } else {
        rule.peers.iter().map(ToString::to_string).collect()
    };
    let prefix = ids.rule;
    v_flex()
        .debug_selector(move || format!("{prefix}-{index}"))
        .w_full()
        .gap(u(gpui::px(1.)))
        .pl(u(look.tokens.spacing.md))
        .border_l_2()
        .border_color(look.tokens.colors.border_variant)
        .child(field(
            look,
            "net-policy-peers",
            ids.peers,
            v_flex().children(peers.into_iter().map(|p| {
                // The text is in the selector too, so a test can tell what reached the screen.
                let tag = p.clone();
                div()
                    .debug_selector(move || format!("net-policy-peer:{tag}"))
                    .child(p)
            })),
        ))
        .child(field(look, "net-policy-ports", "Ports", rule.ports_text()))
        .into_any_element()
}
