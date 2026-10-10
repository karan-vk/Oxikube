//! The command palette (E11-S03): every registered `Command`, searchable, with its key binding,
//! recent ones first.
//!
//! Opens with `cmd-shift-p` (`ctrl-shift-p` on Linux and Windows), the `palette::Toggle` action
//! and command. It is a [`Picker`](crate::Picker) with a [`CommandPaletteDelegate`], hosted in the
//! workspace's modal layer as a [`CommandPalette`] that carries the `Palette` key context.
//!
//! | File | Holds |
//! |---|---|
//! | `host.rs` | [`PaletteHost`]: one per window, the doors that open it ([`register_commands`], the action), [`PaletteRequest`] |
//! | `view.rs` | [`CommandPalette`]: the modal with the `Palette` key context and `palette::ToggleShowAll` |
//! | `delegate.rs` | [`CommandPaletteDelegate`]: matching, ordering, confirm |
//! | `rows.rs` | [`Snapshot`], [`Row`]: the commands classified for the context, the order of matches |
//! | `render.rs` | rows (category chip, title, key caps or the reason) and the footer |
//! | `capture.rs`, `env.rs` | [`capture`]: where the user is when it opens; [`PaletteEnv`]: the session lookup |
//!
//! # What it lists
//!
//! The commands that can run where the palette opened (`CommandIndex` + `CommandInfo::check`): the
//! focused view, the session's capabilities and read-only flag, the selection. The others are
//! **hidden**, not greyed, unless "Show all" is on (`palette::ToggleShowAll`, or the footer): then
//! they are listed dimmed with why they cannot run ("This cluster is read-only"), and confirming
//! one does nothing. The context is read once, when the palette opens: a command acts on what the
//! user had selected then.
//!
//! # Order
//!
//! With nothing typed: the commands run lately first (most recent first), then the rest by
//! category and title. With a query: best fuzzy score first, and among equal scores the recent
//! ones first. The query matches "category title", so `pod shell` finds Shell in Pod.
//!
//! # Running
//!
//! Confirm turns the command id into `Command`s ([`oxikube_app::commands_for`]: one per selected
//! object, operands from the focused view) and sends them through the window's
//! [`CommandDispatcher`](oxikube_workspace::CommandDispatcher), the path keys and buttons take, so
//! the `MutationGuard`, its confirmation and the audit apply as always; the palette never confirms
//! for anyone. A command that needs an operand it cannot ask for (a replica count) says so in a
//! toast: it has its own dialog. The commands run after the palette has closed and handed the
//! focus back, so they act on the same view.
//!
//! # Speed
//!
//! The first frame lists the commands (no matching for an empty query); a keystroke matches on the
//! UI thread up to [`INLINE_MATCH_LIMIT`](crate::picker::fuzzy::INLINE_MATCH_LIMIT) candidates and on the
//! background executor above it; the list is virtualised.

mod capture;
mod delegate;
mod env;
mod host;
mod render;
mod rows;
mod view;

#[cfg(test)]
mod tests;

pub use capture::{Captured, capture, selection_of};
pub use delegate::{CommandPaletteDelegate, Outbox, PaletteParts};
pub use env::PaletteEnv;
pub use host::{PaletteHost, PaletteRequest, PaletteSink, host_of, register_commands};
pub use rows::{Found, Row, Snapshot};
pub use view::CommandPalette;

use gpui::actions;

actions!(
    palette,
    [
        /// Open the command palette, or close it when it is open (`palette::Toggle`).
        Toggle,
        /// List the commands that cannot run here too, or hide them again
        /// (`palette::ToggleShowAll`). Bound in the `Palette` key context.
        ToggleShowAll,
    ]
);

/// The action name of [`ToggleShowAll`], for the footer's key hint.
pub(crate) const TOGGLE_SHOW_ALL_ACTION: &str = "palette::ToggleShowAll";

/// Registers the `palette::Toggle` action handler. Call once at start-up, after the keymap; the
/// binary then installs a [`PaletteHost`] per window.
pub fn init(cx: &mut gpui::App) {
    host::register_actions(cx);
}
