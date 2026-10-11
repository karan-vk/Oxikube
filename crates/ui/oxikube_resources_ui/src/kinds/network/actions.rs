//! The Network section's row action: "Set as default" on IngressClass rows, in the context menu
//! and the command palette (`ingressclass::SetDefault`).

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};

/// Where "Set as default" sits in an IngressClass row's menu: after the view actions.
const SET_DEFAULT_ORDER: u16 = 30;

/// The row actions this section adds: `ingressclass::SetDefault` for IngressClass rows only,
/// one object at a time. It is a mutation: the guard confirms it (low risk), blocks it on a
/// read-only cluster and audits it; the menu shows it greyed out with the reason there.
pub fn network_row_actions() -> [RowActionSpec; 1] {
    [
        RowActionSpec::new(CommandId::INGRESS_CLASS_SET_DEFAULT, |target| {
            Command::IngressClassSetDefault {
                target: target.clone(),
                clear_previous: true,
            }
        })
        .label("Set as default")
        .kinds(KindFilter::Matching(|kind| {
            &*kind.gvk.group == "networking.k8s.io" && &*kind.gvk.kind == "IngressClass"
        }))
        .order(SET_DEFAULT_ORDER),
    ]
}
