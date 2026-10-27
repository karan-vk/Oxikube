//! The ServiceAccount detail (E17-S04): what an account holds, what it may do and who uses it.
//!
//! | File | Holds |
//! |---|---|
//! | `model` | [`ServiceAccountFacts`] (secrets, image pull secrets, automount from the object's JSON), [`BoundRoles`] / [`BoundRole`] (the roles bound to the account, read from the app's `SubjectIndex`) |
//! | `panel` | [`ServiceAccountPanel`]: the detail panel above the Overview, and the poll that recomputes the bound roles on a background task when the index moved |
//! | `render` | the drawing: sections of links |
//! | `bound` | the virtualised bound-roles list |
//! | `kubeconfig` | the **Kubeconfig** section: lifetime, generate (`serviceaccount::GenerateKubeconfig`), the masked preview, copy, save, discard (E17-S09) |
//! | `actions` | [`register_header_action`]: the header's extension point (E17-S09 adds "Generate kubeconfig") |
//!
//! The list columns (Name, Namespace, Secrets, Age, Labels) are the shared catalogue's
//! (`oxikube_app::columns`, kind `ServiceAccount`); the Used by tab and its `u` key are E12-S18's
//! (`oxikube_resources_ui::used_by`), which groups Pods by the top of their owner chains. This
//! module gives the kind its panel and links to that tab.
//!
//! # How a user gets here
//!
//! Open **Access Control / Service Accounts** in the cluster sidebar (or `:sa`), then open a
//! row: the panel sits above the Overview. Its roles come from the cluster's Roles, ClusterRoles,
//! RoleBindings and ClusterRoleBindings, which are subscribed while the detail is open and
//! released with it ([`PolicyHub`](oxikube_app::access::PolicyHub)).

mod actions;
mod bound;
mod kubeconfig;
mod model;
mod panel;
mod render;

use gpui::{AnyView, App, AppContext as _, WeakEntity, Window};
use oxikube_domain::ids::Gvk;
use oxikube_resources_ui::detail::DetailView;
use oxikube_resources_ui::kinds::{DetailRenderer, KindSpecs};

pub use actions::{HeaderAction, header_actions, register_header_action};
pub use kubeconfig::{Expiry, KubeconfigDeps, KubeconfigState, row_actions};
pub use model::{Automount, BoundRole, BoundRoles, ServiceAccountFacts};
pub use panel::ServiceAccountPanel;

/// The ServiceAccount kind.
pub fn gvk() -> Gvk {
    Gvk::new("", "v1", "ServiceAccount")
}

/// Gives the registered ServiceAccount kind its panel. The kind itself is registered by
/// `oxikube_resources_ui` (its Used by action, the sidebar's Access Control entry and the
/// shared columns), which must have run first.
pub fn init(cx: &mut App) {
    let renderer = DetailRenderer::panel(
        |detail: WeakEntity<DetailView>, _: &mut Window, cx: &mut App| -> AnyView {
            cx.new(|cx| ServiceAccountPanel::new(detail, cx)).into()
        },
    );
    if !KindSpecs::set_detail(cx, &gvk(), renderer) {
        tracing::error!("the ServiceAccount kind is not registered; its panel was not installed");
    }
}

#[cfg(test)]
mod tests;
