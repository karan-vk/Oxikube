//! The theme picker (E22-S08, `theme::Select`): every registered theme, grouped dark and light,
//! previewed as the selection moves, kept on Enter, reverted on Escape or a click outside.
//!
//! | File | Holds |
//! |---|---|
//! | `delegate.rs` | [`ThemeDelegate`]: the [`crate::picker::PickerDelegate`] over the registry; previews on selection, commits on confirm, reverts on dismissal |
//! | `render.rs` | the group header, a theme row, the slot caption and the key hints |
//! | `host.rs` | [`ThemeHost`], [`ThemeSink`] and [`register_commands`]: how `theme::Select` from the bus reaches the window |
//!
//! # How a user gets here
//!
//! "Select Theme" in the command palette (`cmd-shift-p`), or an agent calling the `theme_select`
//! tool, runs the `theme::Select` command; the window shows the picker. Typing filters by name.
//!
//! # Preview and commit
//!
//! Moving the selection (arrows, typing, a click on a row) previews that theme with
//! [`oxikube_theme::preview_theme`]: the whole window re-themes at once, and nothing is written
//! (no settings write, so no settings reload). Enter calls [`oxikube_theme::commit_theme`], which
//! edits the user's `settings.json` with the comment-preserving update; the preview ends when the
//! write has landed, so there is no flash of the old theme. Escape, a click outside and closing
//! the window end the preview and the theme the settings resolve to is back.
//!
//! # Which slot a pick edits
//!
//! With `"theme": "Ayu Dark"` a pick replaces that name. With `{ mode, light, dark }` it sets the
//! entry that is in effect now (`mode: system` follows the OS appearance), whatever the picked
//! theme's own appearance, so what the user previewed is what they keep. The caption above the
//! list names the slot ("Editing your dark theme; mode: system"), and the entries of the
//! slot's appearance are listed first.

mod delegate;
mod host;
mod render;
#[cfg(test)]
mod tests;

pub use delegate::{Row, ThemeDelegate};
pub use host::{ThemeHost, ThemeSink, register_commands};
