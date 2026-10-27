//! The RBAC dialogs of the main window (E17-S05): `role::OpenDialog` (the table's "New" on Roles and
//! ClusterRoles, a row's "Edit Rules…", the detail's "Edit rules…", the palette) opens the rule
//! dialog in the cluster's tab. Saving is `role::Create` / `role::Edit`, guarded mutations the
//! bus runs (`oxikube_app::role_ops`). `binding::OpenDialog` (E17-S06) does the same for
//! RoleBindings and ClusterRoleBindings, saving through `binding::Create` / `binding::Edit`
//! (`oxikube_app::binding_ops`).

use std::rc::Rc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Task, WeakEntity, Window};
use oxikube_app::ClusterSessionManager;
use oxikube_rbac_ui::binding::{BindingRequest, BindingViews, BindingViewsDeps};
use oxikube_rbac_ui::role::{RoleRequest, RoleViews, RoleViewsDeps};
use oxikube_workspace::{ClusterTabs, CommandDispatcher};

/// Starts the window's role dialogs over `requests` (the bus's `role::OpenDialog` handler). Keep
/// the task for the life of the window.
pub fn start_views(
    sessions: ClusterSessionManager,
    dispatcher: Rc<dyn CommandDispatcher>,
    tabs: WeakEntity<ClusterTabs>,
    requests: UnboundedReceiver<RoleRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let deps = RoleViewsDeps {
        sessions,
        dispatcher,
        tabs,
    };
    RoleViews::start(deps, requests, window, cx)
}

/// Starts the window's binding dialogs over `requests` (the bus's `binding::OpenDialog` handler).
/// Keep the task for the life of the window.
pub fn start_binding_views(
    sessions: ClusterSessionManager,
    dispatcher: Rc<dyn CommandDispatcher>,
    tabs: WeakEntity<ClusterTabs>,
    requests: UnboundedReceiver<BindingRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let deps = BindingViewsDeps {
        sessions,
        dispatcher,
        tabs,
    };
    BindingViews::start(deps, requests, window, cx)
}
