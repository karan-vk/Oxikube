//! The HTTPRoute, GRPCRoute and TLSRoute panel: hostnames, the parents a route attaches to with
//! the status each controller gave (accepted, resolved refs, with reasons), and the rules with
//! their matches, filters and backends.

use gpui::prelude::*;
use gpui::{AnyElement, Context, div};
use oxikube_app::columns::Tone;
use oxikube_domain::view::network::{RouteBackend, RouteParent, RouteRule, RouteSummary};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::u;

use super::panel::NetworkPanel;
use super::widgets::{Look, MAX_ROWS, chip, conditions, field, heading, more_rows, note, shell};

impl NetworkPanel {
    pub(super) fn route(
        &self,
        look: &Look,
        summary: &RouteSummary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accepted = summary.accepted();
        shell(look, "net-route")
            .child(field(
                look,
                "net-route-hostnames",
                "Hostnames",
                summary.hostnames_text(),
            ))
            .child(field(
                look,
                "net-route-accepted",
                "Accepted",
                chip(
                    look,
                    "net-route-accepted-chip",
                    accepted.as_str(),
                    Tone::of_condition(accepted),
                ),
            ))
            .child(heading(
                look,
                "net-route-parents",
                "Parents",
                Some(summary.parents.len()),
            ))
            .children(
                summary
                    .parents
                    .iter()
                    .take(MAX_ROWS)
                    .enumerate()
                    .map(|(i, p)| self.parent(look, i, p, cx)),
            )
            .child(heading(
                look,
                "net-route-rules",
                "Rules",
                Some(summary.rules.len()),
            ))
            .children(
                summary
                    .rules
                    .iter()
                    .take(MAX_ROWS)
                    .enumerate()
                    .map(|(i, r)| self.rule(look, i, r, cx)),
            )
            .children(more_rows(
                look,
                summary.rules.len().saturating_sub(MAX_ROWS),
            ))
            .into_any_element()
    }

    /// A parent: the link to the Gateway (and its listener), the verdict and its reasons.
    fn parent(
        &self,
        look: &Look,
        index: usize,
        parent: &RouteParent,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accepted = parent.accepted();
        v_flex()
            .debug_selector(move || format!("net-route-parent-{index}"))
            .w_full()
            .child(
                h_flex()
                    .gap(u(look.tokens.spacing.md))
                    .items_center()
                    .child(
                        self.object_link(
                            look,
                            ("net-route-parent-link", index),
                            format!("net-route-parent-link-{index}"),
                            parent.label(),
                            &parent.target,
                            cx,
                        )
                        .flex_1(),
                    )
                    .child(chip(
                        look,
                        "net-route-parent-accepted",
                        accepted.as_str(),
                        Tone::of_condition(accepted),
                    )),
            )
            .children(parent.controller.as_ref().map(|c| {
                note(
                    look,
                    "net-route-parent-controller",
                    format!("controller {c}"),
                )
            }))
            .child(v_flex().pl(u(look.tokens.spacing.lg)).children(conditions(
                look,
                "net-route-parent-condition",
                &parent.conditions,
            )))
            .into_any_element()
    }

    /// A rule: what it matches, the filters it applies and the backends, as links to Services.
    fn rule(
        &self,
        look: &Look,
        index: usize,
        rule: &RouteRule,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let matches = if rule.matches.is_empty() {
            vec!["any request".to_owned()]
        } else {
            rule.matches.clone()
        };
        v_flex()
            .debug_selector(move || format!("net-route-rule-{index}"))
            .w_full()
            .gap(u(gpui::px(1.)))
            .pl(u(look.tokens.spacing.md))
            .border_l_2()
            .border_color(look.tokens.colors.border_variant)
            .child(field(
                look,
                "net-route-matches",
                "Matches",
                v_flex().children(matches.into_iter().map(|m| div().child(m))),
            ))
            .children((!rule.filters.is_empty()).then(|| {
                field(
                    look,
                    "net-route-filters",
                    "Filters",
                    rule.filters.join(", "),
                )
            }))
            .child(field(
                look,
                "net-route-backends",
                "Backends",
                v_flex().children(
                    rule.backends
                        .iter()
                        .enumerate()
                        .map(|(i, b)| self.backend(look, index * 1000 + i, b, cx)),
                ),
            ))
            .into_any_element()
    }

    fn backend(
        &self,
        look: &Look,
        index: usize,
        backend: &RouteBackend,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.object_link(
            look,
            ("net-route-backend", index),
            format!("net-route-backend-{index}"),
            backend.label(),
            &backend.target,
            cx,
        )
        .into_any_element()
    }
}
