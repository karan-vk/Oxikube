//! The help overlay (`?`, E11-S10): the keys that work where the focus is, grouped by category,
//! searchable, with the ones the user (or the vim base keymap) changed marked.
//!
//! Like k9s's `?`, and Zed's keymap views, it is read-only: it lists bindings and runs nothing.
//! `help::Show` is a command (non-negotiable 4) with an MCP tool stub; the `?` key, the command
//! palette and an agent all end in [`HelpHost::toggle`].
//!
//! | File | Holds |
//! |---|---|
//! | `entry.rs` | [`HelpEntry`], [`HelpCategory`], [`HelpSource`], [`HelpState`]: one binding as data; titles and categories from the command registry |
//! | `model.rs` | [`HelpModel`]: the bindings in force along the focus path (`oxikube_keymap::active_bindings`, the ones a `null` hid, or every binding when nothing is focused), sorted, grouped and filtered into [`Row`]s; pure |
//! | `delegate.rs` | [`HelpDelegate`]: the S02 picker delegate (search field, keyboard selection, virtualised list; headers are unselectable rows) |
//! | `render.rs` | the header and binding rows |
//! | `overlay.rs` | [`HelpOverlay`]: the modal (role `Dialog`, key context `Help`) |
//! | `host.rs` | [`HelpHost`]: one per window; the `help::Show` action and bus command |
//!
//! Which bindings are listed comes from the keymap's own dispatch rules
//! ([`oxikube_keymap::active_bindings`]) applied to `window.context_stack()` at the moment the
//! overlay opens, so the list is exactly what the keys do right then, and a `keymap.json` change
//! shows the next time it opens. It is computed once per open, never per frame.
//!
//! `?` opens it from a cluster tab wherever no text field has the focus (the default keymaps null
//! it in the terminal, the manifest editor, the palette and any input), and closes it again while
//! the search field is empty (`"Help && empty > Input"`); Escape always closes. Closing hands the
//! focus back to the view that had it (the modal layer does that).

mod delegate;
mod entry;
mod host;
mod model;
pub(crate) mod overlay;
pub(crate) mod render;
#[cfg(test)]
mod tests;

use gpui::{App, actions};

pub use delegate::HelpDelegate;
pub use entry::{HelpCategory, HelpEntry, HelpSource, HelpState, describe};
pub use host::{HelpHost, HelpSink, register_commands};
pub use model::{HelpModel, HelpScope, Row};
pub use overlay::{ACCESSIBLE_NAME, HelpOverlay};

actions!(
    help,
    [
        /// Open the help overlay over the focused view, or close it when it is open
        /// (`help::Show`, the `?` key).
        Show
    ]
);

/// Binds the `help::Show` action to the overlay of the active window. Call once at start-up, with
/// the other feature crates' `init`.
pub fn init(cx: &mut App) {
    host::register_actions(cx);
}
