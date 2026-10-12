//! `oxikube_overview_ui` — layer: `ui`.
//!
//! Cluster overview dashboard, charts, pulses.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`pulses`] | E13-S10 | the pulses page (`:pulses`): a gauge and a short history per core kind and the cluster's CPU and memory, keyboard-driven, each tile drilling into its table |
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod pulses;

/// Registers what the crate's views need: the pulses sidebar entry. Idempotent. Call it after the
/// crates whose sidebar sections it joins (`oxikube_settings_ui`).
pub fn init(cx: &mut gpui::App) {
    pulses::init(cx);
}
