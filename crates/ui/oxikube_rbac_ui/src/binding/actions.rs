//! The row action of a RoleBinding or ClusterRoleBinding: "Edit Binding…".

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::Gvk;
use oxikube_domain::rbac::RbacKind;

/// Where "Edit Binding…" sits in a binding's menu: after the editor's own entries.
pub const EDIT_BINDING_ORDER: u16 = 37;

/// Whether `gvk` is a RoleBinding or ClusterRoleBinding.
pub fn is_binding_kind(gvk: &Gvk) -> bool {
    matches!(
        RbacKind::of(gvk),
        Some(RbacKind::RoleBinding | RbacKind::ClusterRoleBinding)
    )
}

/// The row actions of the binding kinds, one object at a time.
pub fn binding_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::BINDING_OPEN_DIALOG, |target| {
            Command::BindingOpenDialog {
                cluster: Some(target.cluster.clone()),
                cluster_binding: false,
                target: Some(target.clone()),
            }
        })
        .label("Edit Binding…")
        .kinds(KindFilter::Matching(|kind| is_binding_kind(&kind.gvk)))
        .order(EDIT_BINDING_ORDER),
    ]
}
