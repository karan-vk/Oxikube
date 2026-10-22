//! `oxikube_settings_ui` — layer: `ui`.
//!
//! GUI settings page, keymap editor, theme picker.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`helm_repos`] | E16-S07 | the Helm repositories page as a tab of the active cluster's tab (`helm::ShowRepos`: the palette's "Helm: Repositories", the sidebar's Helm > Repositories entry); the view itself is `oxikube_helm_ui::repos` |
//! | [`metrics`] | E13-S05 | the cluster's metrics page: which Prometheus-compatible service its history graphs come from, what detection found and which services it probed, the `prometheus` settings in effect; `cluster::ShowMetricsSettings` |
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod helm_repos;
pub mod metrics;

/// Registers what the settings pages add to the app: the sidebar's Metrics section and the Helm
/// section's Charts and Repositories entries. Call once at
/// start-up, after `oxikube_workspace`'s own registrations.
pub fn init(cx: &mut gpui::App) {
    metrics::register_sidebar(cx);
    // Charts before Repositories: the catalog is what people open most.
    oxikube_helm_ui::charts::register_sidebar(cx);
    oxikube_helm_ui::upgrade::register_sidebar(cx);
    helm_repos::register_sidebar(cx);
}
