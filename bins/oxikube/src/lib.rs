//! `oxikube` — layer: `bins`.
//!
//! The application's start-up wiring as a library, so tests (and tools such as `xtask`) can run
//! the real init order; `main.rs` is the thin binary around it.
//!
//! - [`startup`]: the documented init order (logging → runtime → assets → settings → theme →
//!   keymap → ui → state db → [`app_state::AppState`] → workspace → features → keymap re-bind),
//!   per-stage timing, and the construction of the adapters that become ports.
//! - [`app_state`]: [`app_state::AppState`], the typed dependency container (a GPUI global), the
//!   [`app_state::AppPorts`] bundle and `AppState::test` (feature `test-support`).
//!
//! Everything else (command line, `--perf`, screenshot and perf scenarios) stays private to the
//! binary.

pub mod app_state;
pub mod startup;
