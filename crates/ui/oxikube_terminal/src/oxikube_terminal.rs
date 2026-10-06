//! `oxikube_terminal` — layer: `ui`.
//!
//! alacritty_terminal grid + custom GPUI Element + TerminalBackend (local PTY, kube exec/attach, display-only).
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
//!
//! # Modules
//!
//! - [`backend`]: [`TerminalBackend`](oxikube_ports::exec::TerminalBackend) implementations.
//!   [`backend::local::LocalPty`] runs the user's shell on a PTY (E09-S02).
//! - [`settings`]: the `terminal` settings ([`TerminalSettings`]).

pub mod backend;
pub mod settings;

pub use settings::TerminalSettings;

/// Registers what this crate puts in the app. The `terminal` settings register themselves
/// (`register_settings!`); the terminal view, its actions and the `terminal::*` commands are
/// added by the stories that build them (E09-S07).
pub fn init(_cx: &mut gpui::App) {}
