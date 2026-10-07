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
//! | [`mappings`] | [`to_esc_str`](mappings::to_esc_str), [`encode_paste`](mappings::encode_paste), [`encode_mouse`](mappings::encode_mouse): keystrokes, pastes and mouse events as the bytes a terminal program expects; pure and table-tested (E09-S06) |
//! | [`input`] | what a focused element attaches: the key-down listener, IME composition (`EntityInputHandler` for [`TerminalState`]), copy / paste / copy on select with the multi-line confirmation, the `terminal::Copy` / `terminal::Paste` commands (E09-S06) |
//! | [`element`] | [`TerminalElement`]: the custom GPUI element painting a terminal (cells, cursor, selection, decorations, links) from theme colours (E09-S05) |
//! | [`open_link`] | `terminal::OpenLink`: the handler cmd/ctrl-click on a link dispatches to (browser URLs; local files opened only when plain, otherwise revealed) (E09-S05) |
//! | [`view`] | [`TerminalView`](view::TerminalView): the terminal as a workspace `Item` (tab title, dirty while running, dockable, saved as its [`BackendDescriptor`](view::BackendDescriptor) only), the cluster's [`TerminalPanel`](view::TerminalPanel) in the bottom dock, and the `terminal::New` / `Split` / `Close` commands (E09-S07) |
//! | [`settings`] | [`TerminalSettings`]: the `terminal` block of `settings.json` (`shell`, `shell_args`, `scrollback_lines`, `copy_on_select`, `option_as_meta`, `confirm_multiline_paste`) |
//!
//! Scrollback is never persisted and terminal bytes are never logged (non-negotiable 5).
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod backend;
pub mod element;
pub mod grid;
pub mod input;
pub mod mappings;
pub mod open_link;
mod quit;
pub mod settings;
pub mod state;
pub mod view;

pub use element::{PathLinks, TerminalElement, TerminalElementState, TerminalFont};
pub use grid::{
    GridMatch, GridPoint, SelectionKind, SelectionSide, TermGrid, TerminalScroll, TerminalSnapshot,
};
pub use settings::TerminalSettings;
pub use state::{TerminalEvent, TerminalState};

/// Registers what this crate puts in the app: the quit hook that deletes the temp kubeconfigs of
/// cluster terminals, the terminal settings with the settings store (they also register through
/// `inventory` when a store is created), the builder of saved terminal tabs and the
/// `terminal::New` / `Split` / `Close` actions ([`view`]). The services terminals start with are
/// installed when the window mounts ([`view::install`]).
pub fn init(cx: &mut gpui::App) {
    use oxikube_settings::Settings as _;
    quit::remove_runtime_dir_on_quit(cx);
    view::init(cx);
    if cx.has_global::<oxikube_settings::SettingsStore>() {
        TerminalSettings::register(cx);
    }
}
