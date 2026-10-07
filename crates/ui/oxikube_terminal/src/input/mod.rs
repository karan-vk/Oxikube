//! Keyboard, IME, clipboard and the input commands of a terminal (E09-S06).
//!
//! The pieces the [`TerminalElement`](crate::TerminalElement) attaches while it is focused:
//!
//! | Module | What |
//! |---|---|
//! | [`keyboard`] | the key-down listener: [`to_esc_str`](crate::mappings::to_esc_str) over the process's modes and the `terminal.option_as_meta` setting; Shift-PageUp/PageDown/Home/End scroll the history; anything that is plain text falls through to the IME handler |
//! | [`ime`] | `EntityInputHandler` for [`TerminalState`]: composition (marked text) shown inline at the cursor, committed text sent as UTF-8, `bounds_for_range` for the candidate window |
//! | [`clipboard`] | `terminal::Copy` / `terminal::Paste`, copy on select, the multi-line paste confirmation ([`PasteConfirm`]) |
//! | [`commands`] | the `terminal::Copy` / `terminal::Paste` bus handlers: palette and agents reach the focused terminal through the window |
//!
//! Everything a user types or pastes goes to the process only: never to a log, the audit trail
//! or disk (non-negotiable 5).

pub mod clipboard;
pub mod commands;
pub mod ime;
pub mod keyboard;

use bytes::Bytes;
use gpui::{Context, actions};

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
            Some(std::borrow::Cow::Borrowed(sequence)) => {
                self.type_bytes(Bytes::from_static(sequence.as_bytes()), cx);
                true
            }
            Some(std::borrow::Cow::Owned(sequence)) => {
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
        let bracketed = self
            .modes()
            .contains(crate::grid::TerminalModes::BRACKETED_PASTE);
        self.type_bytes(encode_paste(text, bracketed), cx);
    }
}
