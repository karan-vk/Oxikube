//! `oxikube_settings_ui` — layer: `ui`.
//!
//! GUI settings page, keymap editor, theme picker.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`metrics`] | E13-S05 | the cluster's metrics page: which Prometheus-compatible service its history graphs come from, what detection found and which services it probed, the `prometheus` settings in effect; `cluster::ShowMetricsSettings` |
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod metrics;

/// Registers what the settings pages add to the app: the sidebar's Metrics section. Call once at
/// start-up, after `oxikube_workspace`'s own registrations.
pub fn init(cx: &mut gpui::App) {
    metrics::register_sidebar(cx);
}
