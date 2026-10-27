//! `oxikube_rbac_ui` — layer: `ui`.
//!
//! RBAC views (SA, roles, bindings), subject-centric matrices, can-i, SA kubeconfig generation.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`can_i`] | E17-S08 | the can-i dialog (`rbac::CanI`): verb, resource, namespace, name and an optional impersonated subject, with the answer, its reason and which review gave it |
//! | [`subjects`] | E17-S07 | the Users and Groups pages and a subject's per-verb policy matrix (`:usr`, `:grp`, `:rbac <subject>`) |
//! | [`binding`] | E17-S06 | RoleBindings and ClusterRoleBindings: the role and subjects (with the rules the role grants) in the detail, the create / edit dialog (a role picker offering only what the binding may name, User / Group / ServiceAccount subjects with the identity string, `cluster-admin` and everybody-group warnings), `binding::OpenDialog` and the row action; saving is the guarded `binding::Create` / `binding::Edit` |
//! | [`role`] | E17-S05 | Roles and ClusterRoles: the rule table in the detail (with aggregation), the create / edit dialog (verb checkboxes, a resource picker from discovery), `role::OpenDialog` and the row action; saving is the guarded `role::Create` / `role::Edit` |
//! | [`service_account`] | E17-S04, E17-S09 | the ServiceAccount detail panel: secrets, image pull secrets, automount, the roles bound to it (from the app's `SubjectIndex`), a link to its Used by tab and the Kubeconfig section (generate, preview, copy, save) |
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod binding;
pub mod can_i;
pub mod role;
pub mod service_account;
pub mod subjects;

/// Registers what this crate puts in the app: the ServiceAccount detail panel and the Role, ClusterRole,
/// RoleBinding and ClusterRoleBinding kinds (their spec and detail panel). Called once from the binary's init, after
/// `oxikube_resources_ui::init`.
pub fn init(cx: &mut gpui::App) {
    service_account::init(cx);
    role::init(cx);
    binding::init(cx);
}

/// Gives the ServiceAccount panel's kubeconfig section what it needs from the app: the filesystem
/// **Save…** writes through (owner-only files) and the process's in-memory vault the
/// `serviceaccount::GenerateKubeconfig` handler fills. Called by the binary once its ports exist.
pub fn install(cx: &mut gpui::App, fs: std::sync::Arc<dyn oxikube_ports::FsPort>) {
    cx.set_global(service_account::KubeconfigDeps {
        fs,
        vault: oxikube_app::sa_kubeconfig::KubeconfigVault::shared(),
    });
}

/// The row actions this crate adds to the resource tables: "Generate Kubeconfig" on ServiceAccounts
/// (`serviceaccount::GenerateKubeconfig` with the default lifetime; the result is in the account's
/// panel).
pub fn row_actions() -> Vec<oxikube_app::RowActionSpec> {
    service_account::row_actions()
}
