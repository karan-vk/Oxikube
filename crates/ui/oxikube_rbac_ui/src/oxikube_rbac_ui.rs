//! `oxikube_rbac_ui` — layer: `ui`.
//!
//! RBAC views (SA, roles, bindings), subject-centric matrices, can-i, SA kubeconfig generation.
//!
//! | Module | Holds |
//! |---|---|
//! | [`can_i`] | the can-i dialog (`rbac::CanI`, E17-S08): verb, resource, namespace, name and an optional impersonated subject, with the answer, its reason and which review gave it |
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`subjects`] | E17-S07 | the Users and Groups pages and a subject's per-verb policy matrix (`:usr`, `:grp`, `:rbac <subject>`) |
//! | [`service_account`] | E17-S04 | the ServiceAccount detail panel: secrets, image pull secrets, automount, the roles bound to it (from the app's `SubjectIndex`) and a link to its Used by tab |

pub mod can_i;
pub mod service_account;
pub mod subjects;

/// Registers what this crate puts in the app: the ServiceAccount detail panel. Called once from
/// the binary's init, after `oxikube_resources_ui::init` (which registers the ServiceAccount kind
/// this gives its panel).
pub fn init(cx: &mut gpui::App) {
    service_account::init(cx);
}
