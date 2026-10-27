//! Roles and ClusterRoles (E17-S05): the rule table in the detail and the rule dialog.
//!
//! | File | Holds |
//! |---|---|
//! | `panel` | [`RolePanel`]: the kind's panel above the Overview. A header (the kind, the rule count, an Aggregated badge, "Edit rules…" and "Delete"), the aggregation selectors and the ClusterRoles they pick, and the rules as a virtualised table of uniform rows: API groups, resources, verbs, resource names, non-resource URLs, normalised for reading with the rule as written on hover |
//! | `rows` | [`RuleRow`]: one rule as the table shows it |
//! | `draft` | [`RoleDraft`]: the dialog's model (the manifest it builds, the problems it finds); no gpui |
//! | `options` | [`ResourceOptions`]: the resource picker's choices from discovery, and which verbs a pick supports |
//! | `dialog`, `dialog_view`, `rule_card`, `rule_editor` | [`RoleDialog`]: rows of rules with a resource picker and verb checkboxes; "Edit as YAML" |
//! | `views` | `role::OpenDialog` ([`register_commands`], [`RoleViews`]) |
//! | `actions` | [`role_row_actions`]: "Edit Rules…" in a Role's row menu and the palette |
//!
//! # Safety
//!
//! Saving is `role::Create` / `role::Edit`, deleting `role::Delete`: guarded mutations at risk
//! medium (`oxikube_app::role_ops`). Nothing here calls a port that writes.
//!
//! # How a user gets here
//!
//! Sidebar > Access Control > Roles or Cluster Roles. A row opens the detail, whose panel is
//! [`RolePanel`]. "New" in the table's toolbar, "Edit Rules…" in a row's menu, "Edit rules…" in the
//! panel and the palette's "Role: New or Edit Rules…" open the dialog.

mod actions;
mod dialog;
mod dialog_view;
mod draft;
mod options;
mod panel;
mod rows;
mod rule_card;
mod rule_editor;
mod views;

#[cfg(test)]
mod tests;

use gpui::{App, AppContext as _};
use oxikube_app::KindMeta;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Scope;
use oxikube_domain::rbac::RbacKind;
use oxikube_resources_ui::kinds::{DetailRenderer, KindSpec, KindSpecs};

pub use actions::{EDIT_RULES_ORDER, is_role_kind, role_row_actions};
pub use dialog::{DialogMode, Load, RoleDialog, RoleDialogDeps};
pub use dialog_view::{LOCKED_NOTE, SAVE_NOTE};
pub use draft::{
    Field, Level, NAMELESS_VERBS, Problem, RoleDraft, RuleDraft, RuleField, VERBS, show_group,
    show_list, split_groups, split_list,
};
pub use options::{ResourceOption, ResourceOptions};
pub use panel::{RolePanel, Sources};
pub use rows::RuleRow;
pub use views::{
    RequestMode, RoleRequest, RoleRequestSink, RoleViews, RoleViewsDeps, register_commands,
};

/// The Role (`plural` `roles`) or ClusterRole kind: the generic table over the shared columns, this
/// panel in the detail, and the dialog among its actions. It has no sidebar slot: the Access
/// Control section already lists both.
pub fn spec(kind: RbacKind) -> KindSpec {
    let (plural, title, scope) = match kind {
        RbacKind::Role => ("roles", "Roles", Scope::Namespaced),
        RbacKind::ClusterRole => ("clusterroles", "Cluster Roles", Scope::Cluster),
        RbacKind::RoleBinding | RbacKind::ClusterRoleBinding => {
            unreachable!("only the role kinds have this panel")
        }
    };
    let meta = KindMeta::new(kind.gvk(), plural, title, scope)
        .with_actions([CommandId::ROLE_OPEN_DIALOG, CommandId::ROLE_DELETE]);
    KindSpec::new(meta).with_detail(DetailRenderer::panel(|detail, _, cx| {
        cx.new(|cx| RolePanel::new(detail, cx)).into()
    }))
}

/// Registers the Role and ClusterRole kinds.
pub fn init(cx: &mut App) {
    for kind in [RbacKind::Role, RbacKind::ClusterRole] {
        if let Err(error) = KindSpecs::register(cx, spec(kind)) {
            tracing::error!(%error, kind = kind.as_str(), "an RBAC role kind was not registered");
        }
    }
}
