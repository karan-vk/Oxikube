//! Keyboard, IME, clipboard and the input commands of a terminal (E09-S06, E09-S11).
//!
//! The pieces the [`TerminalElement`](crate::TerminalElement) attaches while it is focused:
//!
//! | Module | What |
//! |---|---|
//! | [`keyboard`] | the key-down listener: [`to_esc_str`] over the process's modes and the `terminal.option_as_meta` setting; Shift-PageUp/PageDown/Home/End scroll the history; anything that is plain text falls through to the IME handler |
//! | [`ime`] | `EntityInputHandler` for [`TerminalState`]: composition (marked text) shown inline at the cursor, committed text sent as UTF-8, `bounds_for_range` for the candidate window |
//! | [`clipboard`] | `terminal::Copy` / `terminal::Paste`, copy on select, the multi-line paste confirmation ([`PasteConfirm`]) |
//! | [`commands`] | the bus handlers of the terminal's own commands (copy, paste, select all, clear, scroll, search): palette and agents reach the focused terminal through the window (E09-S11 added all but copy and paste) |
//!
//! # Which keys the terminal swallows and which it forwards
//!
//! A terminal in focus sends every keystroke to its process except these (the defaults of
//! `keymap.json`'s `Terminal` context; rebind them there):
//!
//! | | macOS | Linux / Windows |
//! |---|---|---|
//! | copy / paste | `cmd-c`, `cmd-v` | `ctrl-shift-c`, `ctrl-shift-v` (`shift-insert` pastes) |
//! | select all | `cmd-a` | `ctrl-shift-a` |
//! | search; next / previous match | `cmd-f`; `cmd-g`, `cmd-shift-g` | `ctrl-shift-f`; `f3`, `shift-f3` (while the search bar is open) |
//! | clear | `cmd-k` | `ctrl-shift-k` |
//! | new, split, close | `cmd-t`, `cmd-d`, `cmd-w` | `ctrl-shift-t`, `ctrl-shift-d`, `ctrl-shift-w` |
//! | scroll history | `shift-pageup`, `shift-pagedown`, `shift-up`, `shift-down`, `shift-home`, `shift-end` | the same |
//!
//! `ctrl-c`, `ctrl-d`, `ctrl-z`, `ctrl-r`, `ctrl-a` and every other plain `ctrl-` chord reach the
//! process: nothing here binds them.
//!
//! The same holds for the window's own shortcuts (E09-U560). Off macOS they live on
//! `ctrl-shift-<key>`, the application namespace (`ctrl-shift-w` closes the tab, `ctrl-shift-b` /
//! `-j` / `-r` toggle the docks, `ctrl-shift-k <arrow>` splits, `ctrl-shift-q` quits,
//! `ctrl-shift-1..9` switches cluster tab), never on the plain `ctrl-` chords the table above
//! forwards (`ctrl-w`, `ctrl-k`, `ctrl-b`, `ctrl-j`, `ctrl-q`, `ctrl-2..8`, `ctrl--`). The UI
//! zoom chords (`ctrl-=`, `ctrl--`, `ctrl-0`) are unbound (`null`) in the `Terminal` context. `ctrl--` is readline's
//! undo (`0x1f`). The `keymap_shadowing` test (`tests/element`) sweeps every key
//! [`to_esc_str`] encodes against the keymap an off-macOS build
//! installs and fails when a binding shadows one, so a new global shortcut cannot regress this.
//!
//! The scroll keys are the terminal's only on the primary screen; a full-screen program (vim, htop, less) on the alternate screen receives them.
//!
//! Everything a user types or pastes goes to the process only: never to a log, the audit trail
//! or disk (non-negotiable 5).

pub mod clipboard;
pub mod commands;
pub mod ime;
pub mod keyboard;

use std::borrow::Cow;

use bytes::Bytes;
use gpui::{Context, actions};

use crate::grid::TerminalModes;
use crate::mappings::{KeyMode, encode_paste, to_esc_str};
use crate::state::TerminalState;

pub use clipboard::{PasteConfirm, WorkspacePasteConfirm};
pub use commands::{TerminalInputCommand, TerminalInputSink, register_input_commands, run};
pub use ime::ImeAnchor;

/// The key context the element sets: the keymap's `Terminal` sections apply while a terminal is
/// focused.
pub const KEY_CONTEXT: &str = "Terminal";

actions!(
    terminal,
    [
        /// Copy the selection to the clipboard (`cmd-c`, `ctrl-shift-c`). `ctrl-c` is never
        /// copy: it always reaches the process.
        Copy,
        /// Paste the clipboard (`cmd-v`, `ctrl-shift-v`).
        Paste,
        /// Select the whole screen and scrollback (`cmd-a`, `ctrl-shift-a`).
        SelectAll,
        /// Clear the scrollback and the screen above the cursor (`cmd-k`, `ctrl-shift-k`).
        Clear,
        /// Scroll the history one screen up (`shift-pageup`).
        ScrollPageUp,
        /// Scroll the history one screen down (`shift-pagedown`).
        ScrollPageDown,
        /// Scroll the history one line up (`shift-up`).
        ScrollLineUp,
        /// Scroll the history one line down (`shift-down`).
        ScrollLineDown,
        /// Open the search bar (`cmd-f`, `ctrl-shift-f`).
        Search,
        /// Jump to the next search match.
        SearchNext,
        /// Jump to the previous search match.
        SearchPrevious,
        /// Close the search bar.
        SearchClose,
    ]
);

impl TerminalState {
    /// Sends `bytes` the user typed or pasted: back to the live screen first when the view is
    /// scrolled into the history (typing shows what you type).
    pub fn type_bytes(&mut self, bytes: impl Into<Bytes>, cx: &mut Context<Self>) {
        if self.is_scrolled_back() {
            self.scroll_to_bottom(cx);
        }
        self.input(bytes);
    }

    /// Sends `text` as UTF-8 (a committed IME composition).
    pub fn type_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.type_bytes(Bytes::copy_from_slice(text.as_bytes()), cx);
    }

    /// Sends what `keystroke` means to the process, if it is terminal input by itself (see
    /// [`to_esc_str`]). Returns whether it sent anything; plain text is left to the input handler.
    pub fn type_keystroke(
        &mut self,
        keystroke: &gpui::Keystroke,
        option_as_meta: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let mode = KeyMode::new(self.modes(), option_as_meta);
        match to_esc_str(keystroke, mode) {
            Some(Cow::Borrowed(sequence)) => {
                self.type_bytes(Bytes::from_static(sequence.as_bytes()), cx);
                true
            }
            Some(Cow::Owned(sequence)) => {
                self.type_bytes(Bytes::from(sequence), cx);
                true
            }
            None => false,
        }
    }

    /// Pastes `text`: bracketed when the process turned that mode on (an embedded end marker is
    /// stripped), otherwise with line breaks as carriage returns. The multi-line confirmation is
    /// the caller's ([`clipboard::paste_clipboard`]).
    pub fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let bracketed = self.modes().contains(TerminalModes::BRACKETED_PASTE);
        self.type_bytes(encode_paste(text, bracketed), cx);
    }
}
