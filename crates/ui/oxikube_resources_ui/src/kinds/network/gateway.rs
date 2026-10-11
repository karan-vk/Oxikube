//! The GatewayClass and Gateway panels: controller, class, addresses, listeners with their
//! attached routes and status, and the conditions with the reasons controllers give.

use gpui::prelude::*;
use gpui::{AnyElement, Context, div};
use oxikube_app::columns::Tone;
use oxikube_domain::view::ConditionStatus;
use oxikube_domain::view::network::{GatewayClassSummary, GatewaySummary, ListenerSummary};
use oxikube_ui::layout::v_flex;
use oxikube_ui::u;

use super::panel::NetworkPanel;
use super::widgets::{
    Look, MAX_ROWS, cell, chip, conditions, field, heading, more_rows, note, shell, table_header,
    table_row,
};

/// The listener table's columns.
const COLUMNS: [(&str, f32); 5] = [
    ("Name", 0.18),
    ("Protocol", 0.16),
    ("Hostname", 0.28),
    ("Routes from", 0.22),
    ("Attached", 0.16),
];

impl NetworkPanel {
    pub(super) fn gateway_class(&self, look: &Look, summary: &GatewayClassSummary) -> AnyElement {
        let accepted = summary.accepted();
        shell(look, "net-gateway-class")
            .child(field(
                look,
                "net-gc-controller",
                "Controller",
                summary
                    .controller
                    .as_deref()
                    .unwrap_or("not set")
                    .to_owned(),
            ))
            .children(
                summary
                    .description
                    .as_ref()
                    .map(|d| field(look, "net-gc-description", "Description", d.to_string())),
            )
            .children(summary.parameters.as_ref().map(|p| {
                field(
                    look,
                    "net-gc-parameters",
                    "Parameters",
                    format!("{} {}", p.kind, p.label()),
                )
            }))
            .child(field(
                look,
                "net-gc-accepted",
                "Accepted",
                chip(
                    look,
                    "net-gc-accepted-chip",
                    accepted.as_str(),
                    Tone::of_condition(accepted),
                ),
            ))
            .child(heading(look, "net-gc-conditions", "Conditions", None))
            .children(conditions(look, "net-gc-condition", &summary.conditions))
            .into_any_element()
    }

    pub(super) fn gateway(
        &self,
        look: &Look,
        summary: &GatewaySummary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let class = match (summary.class_ref(), summary.class.as_deref()) {
            (Some(target), Some(name)) => self
                .object_link(
                    look,
                    "net-gateway-class-link",
                    "net-gateway-class-link".to_owned(),
                    name.to_owned(),
                    &target,
                    cx,
                )
                .into_any_element(),
            _ => note(look, "net-gateway-no-class", "not set").into_any_element(),
        };
        let addresses = if summary.addresses.is_empty() {
            note(look, "net-gateway-no-addresses", "none assigned yet").into_any_element()
        } else {
            let requested = summary.addresses_requested;
            v_flex()
                .debug_selector(|| "net-gateway-addresses".to_owned())
                .children(
                    summary
                        .addresses
                        .iter()
                        .map(|a| div().child(format!("{} ({})", a.value, a.kind))),
                )
                .when(requested, |el| {
                    el.child(note(
                        look,
                        "net-gateway-requested",
                        "requested in the spec, not assigned yet",
                    ))
                })
                .into_any_element()
        };
        let status = |id: &'static str, status: ConditionStatus| {
            chip(look, id, status.as_str(), Tone::of_condition(status))
        };
        shell(look, "net-gateway")
            .child(field(look, "net-gateway-class", "Class", class))
            .child(field(look, "net-gateway-addr", "Addresses", addresses))
            .child(field(
                look,
                "net-gateway-accepted",
                "Accepted",
                status("net-gateway-accepted-chip", summary.accepted()),
            ))
            .child(field(
                look,
                "net-gateway-programmed",
                "Programmed",
                status("net-gateway-programmed-chip", summary.programmed()),
            ))
            .child(heading(
                look,
                "net-gateway-listeners",
                "Listeners",
                Some(summary.listeners.len()),
            ))
            .child(table_header(look, "net-gateway-header", &COLUMNS))
            .children(
                summary
                    .listeners
                    .iter()
                    .take(MAX_ROWS)
                    .enumerate()
                    .map(|(i, l)| listener(look, i, l)),
            )
            .children(more_rows(
                look,
                summary.listeners.len().saturating_sub(MAX_ROWS),
            ))
            .child(heading(look, "net-gateway-conditions", "Conditions", None))
            .children(conditions(
                look,
                "net-gateway-condition",
                &summary.conditions,
            ))
            .into_any_element()
    }
}

/// One listener: its row, then the conditions its controller reported for it.
fn listener(look: &Look, index: usize, listener: &ListenerSummary) -> AnyElement {
    let [name, protocol, host, from, attached] = COLUMNS.map(|(_, fraction)| fraction);
    let protocol_text = match (listener.port, &listener.tls_mode) {
        (Some(port), Some(mode)) => format!("{} :{port} ({mode})", listener.protocol),
        (Some(port), None) => format!("{} :{port}", listener.protocol),
        (None, _) => listener.protocol.clone(),
    };
    v_flex()
        .debug_selector(move || format!("net-listener-{index}"))
        .w_full()
        .child(
            table_row(format!("net-listener-row-{index}"))
                .child(cell(name, listener.name.clone()))
                .child(cell(protocol, protocol_text))
                .child(cell(
                    host,
                    listener.hostname.clone().unwrap_or_else(|| "*".to_owned()),
                ))
                .child(cell(from, listener.allowed_namespaces.clone()))
                .child(cell(
                    attached,
                    listener
                        .attached_routes
                        .map_or_else(|| "-".to_owned(), |n| n.to_string()),
                )),
        )
        .children((!listener.conditions.is_empty()).then(|| {
            v_flex().pl(u(look.tokens.spacing.lg)).children(conditions(
                look,
                "net-listener-condition",
                &listener.conditions,
            ))
        }))
        .into_any_element()
}
