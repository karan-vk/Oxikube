//! RoleBindings and ClusterRoleBindings (E17-S06): the role and subjects in the detail and the
//! binding dialog.
//!
//! | File | Holds |
//! |---|---|
//! | `panel` | [`BindingPanel`]: the kind's panel above the Overview. A header (the kind, the role it grants as a link that opens it, the subject count, "Edit…" and "Delete"), the subjects as a virtualised table (kind, name, namespace, apiGroup) and the rules the role grants (the "effective rules") as a second one |
//! | `draft` | [`BindingDraft`]: the dialog's model (the manifest it builds, the problems it finds, the escalation warnings); no gpui |
//! | `dialog`, `dialog_edit`, `dialog_view`, `role_picker`, `subject_row` | [`BindingDialog`]: a role picker that offers only what the binding may name (a ClusterRoleBinding only ClusterRoles, a RoleBinding the Roles of its namespace and every ClusterRole), subject rows (User and Group with autocomplete from the names bindings already use, ServiceAccount with a namespace and a picker), the identity string, "Edit as YAML" |
//! | `views` | `binding::OpenDialog` ([`register_commands`], [`BindingViews`]) |
//! | `actions` | [`binding_row_actions`]: "Edit Binding…" in a binding's row menu and the palette |
//!
//! # Safety
//!
//! Saving is `binding::Create` / `binding::Edit`, deleting `binding::Delete`: guarded mutations
//! at risk medium (`oxikube_app::binding_ops`); the confirmation calls out a grant to
//! `cluster-admin` or to everybody (the domain's `Escalation` table decides, not this crate).
//! Nothing here calls a port that writes.
//!
//! # How a user gets here
//!
//! Sidebar > Access Control > Role Bindings or Cluster Role Bindings. A row opens the detail,
//! whose panel is [`BindingPanel`]. "New" in the table's toolbar, "Edit Binding…" in a row's menu,
//! "Edit…" in the panel and the palette's "Binding: New or Edit Subjects…" open the dialog.

mod actions;
mod dialog;
mod dialog_edit;
mod dialog_view;
mod draft;
mod panel;
mod role_picker;
mod subject_row;
mod views;

#[cfg(test)]
mod tests;

use gpui::{App, AppContext as _};
use oxikube_app::KindMeta;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Scope;
use oxikube_domain::rbac::RbacKind;
use oxikube_resources_ui::kinds::{DetailRenderer, KindSpec, KindSpecs};

pub use actions::{EDIT_BINDING_ORDER, binding_row_actions, is_binding_kind};
pub use dialog::{BindingDialog, BindingDialogDeps, DialogMode};
pub use dialog_edit::SubjectEditor;
pub use dialog_view::{RECREATE_NOTE, SAVE_NOTE};
pub use draft::{BindingDraft, Field, Problem, SubjectDraft, SubjectKind};
pub use panel::{BindingPanel, RolePhase, SubjectRow};
pub use views::{
    BindingRequest, BindingRequestSink, BindingViews, BindingViewsDeps, RequestMode,
    register_commands,
};

/// The RoleBinding or ClusterRoleBinding kind: the generic table over the shared columns (Name,
/// Namespace, Role ref, Subjects, Age), this panel in the detail, and the dialog among its
/// actions. It has no sidebar slot: the Access Control section already lists both.
pub fn spec(kind: RbacKind) -> KindSpec {
    let (plural, title, scope) = match kind {
        RbacKind::RoleBinding => ("rolebindings", "Role Bindings", Scope::Namespaced),
        RbacKind::ClusterRoleBinding => (
            "clusterrolebindings",
            "Cluster Role Bindings",
            Scope::Cluster,
        ),
        RbacKind::Role | RbacKind::ClusterRole => {
            unreachable!("only the binding kinds have this panel")
        }
    };
    let meta = KindMeta::new(kind.gvk(), plural, title, scope)
        .with_actions([CommandId::BINDING_OPEN_DIALOG, CommandId::BINDING_DELETE]);
    KindSpec::new(meta).with_detail(DetailRenderer::panel(|detail, _, cx| {
        cx.new(|cx| BindingPanel::new(detail, cx)).into()
    }))
}

/// Registers the RoleBinding and ClusterRoleBinding kinds.
pub fn init(cx: &mut App) {
    for kind in [RbacKind::RoleBinding, RbacKind::ClusterRoleBinding] {
        if let Err(error) = KindSpecs::register(cx, spec(kind)) {
            tracing::error!(%error, kind = kind.as_str(), "an RBAC binding kind was not registered");
        }
    }
}
