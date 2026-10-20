//! What the keys, the clicks and the toolbar do: move the selection, and send the selected
//! forward's row action on the bus. The commands' effects are the app's (`portforward::Stop`,
//! `Restart`, `Remove`) or the window's ([`serve`](crate::serve): `CopyUrl`, `OpenInBrowser`), so
//! a key, a button, the palette and an agent run one behaviour.

use gpui::{Context, Window};
use oxikube_app::ForwardId;
use oxikube_domain::command::Command;
use oxikube_ui::table::{RowClick, TableEvent};
use oxikube_workspace::panel::PanelEvent;

use super::PortForwardPanel;

impl PortForwardPanel {
    pub(super) fn on_table_event(&mut self, event: &TableEvent, cx: &mut Context<Self>) {
        if let TableEvent::RowClicked(RowClick { row, count, .. }) = event {
            self.select(*row, cx);
            if *count >= 2 {
                self.send_selected(|id| Command::PortForwardOpenInBrowser { id }, cx);
            }
        }
    }

    /// Selects line `ix`.
    pub fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.table.update_quiet(cx, |d| d.model.select(ix)) {
            cx.notify();
        }
    }

    /// Moves the selection `delta` lines and scrolls it into view.
    pub fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(ix) = self.table.update_quiet(cx, |d| d.model.step(delta)) {
            self.table.reveal_row(ix, cx);
            cx.notify();
        }
    }

    /// Selects the first or the last line and scrolls it into view.
    pub fn select_end(&mut self, last: bool, cx: &mut Context<Self>) {
        if let Some(ix) = self.table.update_quiet(cx, |d| d.model.select_end(last)) {
            self.table.reveal_row(ix, cx);
            cx.notify();
        }
    }

    /// The selected forward's id.
    pub fn selected_id(&self, cx: &gpui::App) -> Option<ForwardId> {
        self.model(cx).selected().map(|row| row.id)
    }

    /// Sends `command(id)` for the selected forward; nothing when none is selected.
    pub(super) fn send_selected(&self, command: fn(u64) -> Command, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_id(cx) {
            self.deps.dispatcher.dispatch(command(id.0), cx);
        }
    }

    /// `s`: stops the selected forward, if it is running.
    pub fn stop_selected(&mut self, cx: &mut Context<Self>) {
        if self.model(cx).selected().is_some_and(|row| row.running) {
            self.send_selected(|id| Command::PortForwardStop { id }, cx);
        }
    }

    /// `r`: restarts the selected forward; a stopped or failed one starts again.
    pub fn restart_selected(&mut self, cx: &mut Context<Self>) {
        self.send_selected(|id| Command::PortForwardRestart { id }, cx);
    }

    /// `d`: removes the selected forward from the list (stopping it first).
    pub fn remove_selected(&mut self, cx: &mut Context<Self>) {
        self.send_selected(|id| Command::PortForwardRemove { id }, cx);
    }

    /// `c`: copies the selected forward's URL.
    pub fn copy_url_selected(&mut self, cx: &mut Context<Self>) {
        self.send_selected(|id| Command::PortForwardCopyUrl { id }, cx);
    }

    /// `o`: opens the selected forward in the system browser.
    pub fn open_selected(&mut self, cx: &mut Context<Self>) {
        self.send_selected(|id| Command::PortForwardOpenInBrowser { id }, cx);
    }

    /// `/`: puts the focus in the filter.
    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |input, cx| input.focus(window, cx));
    }

    /// `escape` in the filter: the rows have the focus again.
    pub fn focus_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    /// `escape`: closes the panel.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(PanelEvent::Close);
    }
}
