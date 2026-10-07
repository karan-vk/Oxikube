//! Copy, paste and copy on select.
//!
//! * **Copy** (`terminal::Copy`, `cmd-c` / `ctrl-shift-c`): the selection to the clipboard. With no
//!   selection it does nothing; `ctrl-c` is not a copy binding, so it always reaches the process as
//!   `0x03`.
//! * **Copy on select** (`terminal.copy_on_select`): the same copy when a mouse selection ends,
//!   plus the primary selection on Linux.
//! * **Paste** (`terminal::Paste`, `cmd-v` / `ctrl-shift-v`): the clipboard's text, bracketed when
//!   the process asked ([`TerminalState::paste_text`]). Text with a line break asks first
//!   (`terminal.confirm_multiline_paste`, default on): a pasted newline runs the line. The dialog
//!   comes from the host through [`PasteConfirm`]; [`WorkspacePasteConfirm`] shows it as a modal
//!   on a workspace.
//!
//! Clipboard text is never logged and never kept: it goes to the process or the clipboard.

use std::rc::Rc;

use gpui::{App, AppContext as _, ClipboardItem, Entity, WeakEntity, Window};
use oxikube_workspace::Workspace;
use oxikube_workspace::modal::DialogModal;

use crate::mappings::{is_multiline, preview};
use crate::settings::TerminalSettings;
use crate::state::TerminalState;

/// Asks the user whether to paste text with several lines. Implemented by the host (the terminal
/// view shows a dialog); a test implements it with a recorder.
pub trait PasteConfirm {
    /// Shows `text`'s first lines and runs `accept` if the user confirms. Cancelling does
    /// nothing; nothing is sent either way until `accept` runs.
    fn confirm(
        &self,
        text: &str,
        accept: Rc<dyn Fn(&mut Window, &mut App)>,
        window: &mut Window,
        cx: &mut App,
    );
}

/// [`PasteConfirm`] as a modal dialog on a [`Workspace`] (the dialog layer of the window).
#[derive(Clone)]
pub struct WorkspacePasteConfirm(WeakEntity<Workspace>);

impl WorkspacePasteConfirm {
    /// Confirms through `workspace`'s modal layer.
    pub fn new(workspace: WeakEntity<Workspace>) -> Self {
        Self(workspace)
    }
}

impl PasteConfirm for WorkspacePasteConfirm {
    fn confirm(
        &self,
        text: &str,
        accept: Rc<dyn Fn(&mut Window, &mut App)>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(workspace) = self.0.upgrade() else {
            return;
        };
        let lines = text.lines().count().max(2);
        let title = format!("Paste {lines} lines into the terminal?");
        let message = preview(text);
        workspace.update(cx, |workspace, cx| {
            workspace.show_modal(
                cx.new(|cx| {
                    DialogModal::new(title, cx)
                        .message(message)
                        .confirm_label("Paste")
                        .on_confirm(move |window, cx| accept(window, cx))
                }),
                window,
                cx,
            );
        });
    }
}

/// Copies the selection of `terminal` to the clipboard. Returns whether anything was copied (a
/// selection of text exists).
pub fn copy_selection(terminal: &Entity<TerminalState>, cx: &mut App) -> bool {
    let Some(text) = terminal
        .read(cx)
        .selection_text()
        .filter(|text| !text.is_empty())
    else {
        return false;
    };
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    cx.write_to_primary(ClipboardItem::new_string(text.clone()));
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    true
}

/// Pastes the clipboard into `terminal`, asking `confirm` first for several lines when the
/// setting is on. Without a `confirm` the paste goes ahead: the host that wants the safety net
/// supplies one.
pub fn paste_clipboard(
    terminal: &Entity<TerminalState>,
    confirm: Option<&Rc<dyn PasteConfirm>>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(text) = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .filter(|text| !text.is_empty())
    else {
        return;
    };
    paste_text(terminal, text, confirm, window, cx);
}

/// [`paste_clipboard`] for text the caller already has.
pub fn paste_text(
    terminal: &Entity<TerminalState>,
    text: String,
    confirm: Option<&Rc<dyn PasteConfirm>>,
    window: &mut Window,
    cx: &mut App,
) {
    let ask = TerminalSettings::confirm_multiline_paste(cx) && is_multiline(&text);
    let (true, Some(confirm)) = (ask, confirm) else {
        terminal.update(cx, |terminal, cx| terminal.paste_text(&text, cx));
        return;
    };
    let target = terminal.downgrade();
    let shown = text.clone();
    let accept: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |_, cx| {
        if let Some(terminal) = target.upgrade() {
            terminal.update(cx, |terminal, cx| terminal.paste_text(&text, cx));
        }
    });
    confirm.confirm(&shown, accept, window, cx);
}
