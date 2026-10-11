//! The Ingress panel: class, load balancers, and the rules table (host, path, path type, backend
//! service and port, TLS secret) with a link to each backing Service.

use gpui::prelude::*;
use gpui::{AnyElement, Context};
use oxikube_app::columns::Tone;
use oxikube_domain::view::network::{IngressBackend, IngressRuleRow, IngressSummary, ObjectRef};
use oxikube_ui::layout::v_flex;

use super::panel::NetworkPanel;
use super::widgets::{
    Look, MAX_ROWS, cell, chip, field, heading, more_rows, note, shell, table_header, table_row,
};

/// The rules table's columns: title and share of the width.
const COLUMNS: [(&str, f32); 5] = [
    ("Host", 0.32),
    ("Path", 0.14),
    ("Type", 0.12),
    ("Backend", 0.22),
    ("TLS secret", 0.20),
];

impl NetworkPanel {
    pub(super) fn ingress(
        &self,
        look: &Look,
        summary: &IngressSummary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let class = summary
            .class
            .as_deref()
            .unwrap_or("none (the default class applies)");
        let addresses = if summary.load_balancers.is_empty() {
            "none assigned yet".to_owned()
        } else {
            summary.load_balancers.join(", ")
        };
        let mut rows: Vec<AnyElement> = summary
            .rules
            .iter()
            .take(MAX_ROWS)
            .enumerate()
            .map(|(i, row)| self.rule_row(look, i, row, cx))
            .collect();
        if let Some(backend) = &summary.default_backend {
            rows.push(self.default_backend_row(look, backend, cx));
        }
        shell(look, "net-ingress")
            .child(field(look, "net-ingress-class", "Class", class.to_owned()))
            .child(field(look, "net-ingress-lb", "Load balancers", addresses))
            .child(heading(
                look,
                "net-ingress-rules",
                "Rules",
                Some(summary.rules.len()),
            ))
            .child(if rows.is_empty() {
                note(look, "net-ingress-no-rules", "This Ingress has no rules.").into_any_element()
            } else {
                v_flex()
                    .w_full()
                    .child(table_header(look, "net-ingress-header", &COLUMNS))
                    .children(rows)
                    .into_any_element()
            })
            .children(more_rows(
                look,
                summary.rules.len().saturating_sub(MAX_ROWS),
            ))
            .into_any_element()
    }

    fn rule_row(
        &self,
        look: &Look,
        index: usize,
        row: &IngressRuleRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let [host, path, kind, backend, tls] = COLUMNS.map(|(_, fraction)| fraction);
        let tls_cell = match &row.tls_secret {
            Some(secret) => {
                chip(look, "net-ingress-tls", secret.clone(), Tone::Ok).into_any_element()
            }
            None => note(look, "net-ingress-no-tls", "none").into_any_element(),
        };
        table_row(format!("net-ingress-rule-{index}"))
            .child(cell(
                host,
                row.host.clone().unwrap_or_else(|| "*".to_owned()),
            ))
            .child(cell(path, row.path.clone().unwrap_or_default()))
            .child(cell(kind, row.path_type.clone().unwrap_or_default()))
            .child(cell(
                backend,
                match &row.backend {
                    Some(backend) => {
                        self.backend_link(look, ("net-ingress-backend", index), backend, cx)
                    }
                    None => note(look, "net-ingress-no-backend", "").into_any_element(),
                },
            ))
            .child(cell(tls, tls_cell))
            .into_any_element()
    }

    fn default_backend_row(
        &self,
        look: &Look,
        backend: &IngressBackend,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        table_row("net-ingress-default-backend".to_owned())
            .child(cell(COLUMNS[0].1, "(default backend)"))
            .child(cell(COLUMNS[1].1, ""))
            .child(cell(COLUMNS[2].1, ""))
            .child(cell(
                COLUMNS[3].1,
                self.backend_link(look, ("net-ingress-default-link", 0), backend, cx),
            ))
            .into_any_element()
    }

    /// A Service backend as a link to the Service; any other backend as plain text.
    fn backend_link(
        &self,
        look: &Look,
        id: (&'static str, usize),
        backend: &IngressBackend,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(service) = backend.service() else {
            return gpui::div().child(backend.to_string()).into_any_element();
        };
        let target = ObjectRef::new(
            "",
            "Service",
            self.object().and_then(|o| o.namespace()),
            service,
        );
        self.object_link(
            look,
            id,
            format!("{}-{}", id.0, id.1),
            backend.to_string(),
            &target,
            cx,
        )
        .into_any_element()
    }
}
