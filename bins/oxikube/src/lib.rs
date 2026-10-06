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
//! - [`cluster_prefs`]: pushes the per-cluster settings into the `ClusterSessionManager` and
//!   keeps them in sync with hot reload (E06-S08).
//! - [`kube_ports`]: the cluster adapters of the app (`oxikube_kube`: the kubeconfig catalog,
//!   built on first use, and the connector), the system clock (E07-S00).
//! - [`mount`]: the cluster UI in the main window: catalog home, hotbar, cluster tabs with their
//!   sidebar, connect views and namespace selector, the status bar item, the command bus, session
//!   restore (E07-S00).
//! - [`perf_table`]: what `oxikube --perf-table` does in the window: connect a context, open its
//!   pods table and scroll it, through the same commands as a user (E07-S09).
//!
//! Everything else (command line, the `--perf` session, screenshot and perf scenarios) stays
//! private to the binary.

pub mod app_state;
pub mod cluster_prefs;
pub mod kube_ports;
pub mod mount;
pub mod perf_table;
pub mod startup;
