//! The row actions of a Service: "Show Pods" (`service::ShowPods`, k9s-style `u`), "Open in
//! Browser" (`service::OpenInBrowser`) and "Port-Forward…" (opens the start dialog, E15-S04).
//!
//! The binary registers them next to the other kinds' (`bins/oxikube`). Both are offered for
//! Services only. Like every row action they appear only when their command is on the bus: "Show
//! Pods" always is ([`register_commands`](crate::register_commands)); "Open in Browser" only once a
//! port-forward manager installed its opener ([`register_open_in_browser`](super::register_open_in_browser)).

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::portforward::ForwardPort;

/// Where "Show Pods" sits in a Service's menu: after "View YAML" and "Describe", before the
/// generic actions.
pub const SHOW_PODS_ORDER: u16 = 125;
/// Where "Open in Browser" sits: right after "Show Pods".
pub const OPEN_IN_BROWSER_ORDER: u16 = 126;
/// Where "Port-Forward…" sits: right after "Open in Browser".
pub const PORT_FORWARD_ORDER: u16 = 127;

/// The row actions this module adds, for Services.
pub fn service_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::SERVICE_SHOW_PODS, |target| {
            Command::ServiceShowPods {
                target: target.clone(),
            }
        })
        .label("Show Pods")
        .kinds(KindFilter::Matching(|kind| kind.gvk.is_service()))
        .order(SHOW_PODS_ORDER),
        // No port: the opener forwards the first TCP port; the panel's buttons name one.
        RowActionSpec::new(CommandId::SERVICE_OPEN_IN_BROWSER, |target| {
            Command::ServiceOpenInBrowser {
                target: target.clone(),
                port: None,
            }
        })
        .label("Open in Browser")
        .kinds(KindFilter::Matching(|kind| kind.gvk.is_service()))
        .order(OPEN_IN_BROWSER_ORDER),
        // The table opens the start dialog, which sends `portforward::Start` when it is
        // submitted (E15-S04); this builder serves a caller that has only a row.
        RowActionSpec::new(CommandId::PORTFORWARD_START, |target| {
            Command::PortForwardStart {
                target: target.clone(),
                remote_port: ForwardPort::Number(0),
                local_port: None,
                bind: None,
                name: None,
                favourite: false,
            }
        })
        .label("Port-Forward…")
        .kinds(KindFilter::Matching(|kind| kind.gvk.is_service()))
        .order(PORT_FORWARD_ORDER),
    ]
}
