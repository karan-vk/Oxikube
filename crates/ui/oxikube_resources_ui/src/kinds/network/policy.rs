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

/// One direction: its heading with the effect chip, the sentence about it, then its rules.
fn direction(look: &Look, summary: &NetworkPolicySummary, which: PolicyType) -> AnyElement {
    let (rules, id, noun) = match which {
        PolicyType::Ingress => (&summary.ingress, "net-policy-ingress", "ingress"),
        PolicyType::Egress => (&summary.egress, "net-policy-egress", "egress"),
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
    let word = match which {
        PolicyType::Ingress => "From",
        PolicyType::Egress => "To",
    };
    v_flex()
        .debug_selector(move || id.to_owned())
        .gap(u(look.tokens.spacing.sm))
        .child(heading(look, id_heading(which), noun, Some(rules.len())))
        .child(chip(look, id_chip(which), label, tone))
        .child(note(look, id_note(which), sentence))
        .children(
            rules
                .iter()
                .take(MAX_ROWS)
                .enumerate()
                .map(|(i, rule)| rule_block(look, which, i, word, rule)),
        )
        .children(more_rows(look, rules.len().saturating_sub(MAX_ROWS)))
        .into_any_element()
}

fn id_heading(which: PolicyType) -> &'static str {
    match which {
        PolicyType::Ingress => "net-policy-ingress-heading",
        PolicyType::Egress => "net-policy-egress-heading",
    }
}

fn id_chip(which: PolicyType) -> &'static str {
    match which {
        PolicyType::Ingress => "net-policy-ingress-effect",
        PolicyType::Egress => "net-policy-egress-effect",
    }
}

fn id_note(which: PolicyType) -> &'static str {
    match which {
        PolicyType::Ingress => "net-policy-ingress-note",
        PolicyType::Egress => "net-policy-egress-note",
    }
}

/// One rule: its peers, one per line, and its ports.
fn rule_block(
    look: &Look,
    which: PolicyType,
    index: usize,
    word: &str,
    rule: &PolicyRule,
) -> AnyElement {
    let peers: Vec<String> = if rule.peers.is_empty() {
        vec![rule.peers_text()]
    } else {
        rule.peers.iter().map(ToString::to_string).collect()
    };
    let prefix = match which {
        PolicyType::Ingress => "net-policy-ingress-rule",
        PolicyType::Egress => "net-policy-egress-rule",
    };
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
            word,
            v_flex().children(peers.into_iter().map(|p| div().child(p))),
        ))
        .child(field(look, "net-policy-ports", "Ports", rule.ports_text()))
        .into_any_element()
}
