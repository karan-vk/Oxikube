//! The row action of a Role or ClusterRole: "Edit Rules…".

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::Gvk;
use oxikube_domain::rbac::RbacKind;

/// Where "Edit Rules…" sits in a Role's menu: after the editor's own entries.
pub const EDIT_RULES_ORDER: u16 = 36;

/// Whether `gvk` is a Role or ClusterRole.
pub fn is_role_kind(gvk: &Gvk) -> bool {
    matches!(
        RbacKind::of(gvk),
        Some(RbacKind::Role | RbacKind::ClusterRole)
    )
}

/// The row actions of the Role kinds, one object at a time.
pub fn role_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::ROLE_OPEN_DIALOG, |target| {
            Command::RoleOpenDialog {
                cluster: Some(target.cluster.clone()),
                cluster_role: false,
                target: Some(target.clone()),
            }
        })
        .label("Edit Rules…")
        .kinds(KindFilter::Matching(|kind| is_role_kind(&kind.gvk)))
        .order(EDIT_RULES_ORDER),
    ]
}
