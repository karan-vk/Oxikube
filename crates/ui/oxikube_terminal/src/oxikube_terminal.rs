//! `oxikube_terminal` — layer: `ui`.
//!
//! One terminal for local shells, pod exec/attach, node shells and debug containers (epic E09):
//! an `alacritty_terminal` grid painted by our own GPUI element, fed by any
//! [`TerminalBackend`](oxikube_ports::TerminalBackend).
//!
//! | Module | What |
//! |---|---|
//! | [`backend`] | [`TerminalBackend`](oxikube_ports::exec::TerminalBackend) implementations. [`backend::local::LocalPty`] runs the user's shell on a PTY (E09-S02) |
//! | [`grid`] | [`TermGrid`]: `alacritty_terminal` (pinned `=0.26.0`) behind our own types; the only module that names them. Snapshot, selection, search, scrollback (E09-S04) |
//! | [`state`] | [`TerminalState`]: the GPUI entity bridging a backend and the grid; tokio pump, writer, frame-coalesced notify (E09-S04) |
//! | [`settings`] | [`TerminalSettings`]: the `terminal` block of `settings.json` (`shell`, `shell_args`, `scrollback_lines`) |
//!
//! Scrollback is never persisted and terminal bytes are never logged (non-negotiable 5).
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod backend;
pub mod grid;
mod quit;
pub mod settings;
pub mod state;

pub use grid::{
    GridMatch, GridPoint, SelectionKind, SelectionSide, TermGrid, TerminalScroll, TerminalSnapshot,
};
pub use settings::TerminalSettings;
pub use state::{TerminalEvent, TerminalState};

/// Registers what this crate puts in the app: the quit hook that deletes the temp kubeconfigs of
/// cluster terminals, and the terminal settings with the settings store (they also register
/// through `inventory` when a store is created). The terminal view, its actions and the
/// `terminal::*` commands are added by the stories that build them (E09-S07).
pub fn init(cx: &mut gpui::App) {
    use oxikube_settings::Settings as _;
    quit::remove_runtime_dir_on_quit(cx);
    if cx.has_global::<oxikube_settings::SettingsStore>() {
        TerminalSettings::register(cx);
    }
}
