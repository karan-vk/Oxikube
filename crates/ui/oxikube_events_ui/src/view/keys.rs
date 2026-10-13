//! What the keys, the clicks and the commands do: move the selection, open the event's object,
//! copy a row, export the listed events. The commands' own effects (`events::CopyRow`, ...) are
//! here too; the actions dispatch the command and the bus hands it back through
//! [`EventsViews`](crate::EventsViews), so a key, the palette and an agent run one behaviour.

use std::path::PathBuf;

use gpui::{ClipboardItem, Context, Window};
use oxikube_domain::command::Command;
use oxikube_runtime::spawn_kube;
use oxikube_ui::table::{RowClick, TableEvent};
use oxikube_workspace::Toast;

use super::EventsView;
use crate::export::{export_json, row_json, row_text, suggested_file_name};

impl EventsView {
    pub(super) fn on_table_event(&mut self, event: &TableEvent, cx: &mut Context<Self>) {
        match event {
            TableEvent::RowClicked(click) => self.on_row_click(*click, cx),
            // The delegate took the new order in `perform_sort`.
            TableEvent::SortChanged { .. } => cx.notify(),
            _ => {}
        }
    }

    fn on_row_click(&mut self, click: RowClick, cx: &mut Context<Self>) {
        self.select(click.row, cx);
        if click.count >= 2 {
            self.open_selected(cx);
        }
    }

    /// Selects row `ix`.
    pub fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.table.update_quiet(cx, |d| d.model.select(ix)) {
            cx.notify();
        }
    }

    /// Moves the selection `delta` rows and scrolls it into view.
    pub fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(ix) = self.table.update_quiet(cx, |d| d.model.step(delta)) {
            self.table.reveal_row(ix, cx);
            cx.notify();
        }
    }

    /// Selects the first or the last row and scrolls it into view.
    pub fn select_end(&mut self, last: bool, cx: &mut Context<Self>) {
        if let Some(ix) = self.table.update_quiet(cx, |d| d.model.select_end(last)) {
            self.table.reveal_row(ix, cx);
            cx.notify();
        }
    }

    /// Opens the selected event's object: `resource::Open` on the bus, the command every table
    /// and the overview's warnings send.
    pub fn open_selected(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.model(cx).selected_row() else {
            return;
        };
        let mut target = row.event.regarding.clone();
        target.cluster = self.cluster.clone();
        self.deps
            .dispatcher
            .dispatch(Command::ResourceOpen { target }, cx);
    }

    /// Sends `command` for this view's cluster on the bus.
    pub(super) fn dispatch(&self, command: Command, cx: &mut Context<Self>) {
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// `events::CopyRow` / `events::CopyRowJson`: the selected row on the clipboard, as one
    /// line of text or as JSON (the message redacted).
    pub fn copy_selected(&mut self, json: bool, cx: &mut Context<Self>) {
        let Some(row) = self.model(cx).selected_row() else {
            self.toast(Toast::info("Select an event to copy."), cx);
            return;
        };
        let text = if json {
            row_json(&row.event)
        } else {
            row_text(&row.event)
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// `events::ExportJson`: asks where, then writes the listed events (in the view's order,
    /// the messages redacted) through the `FsPort` off the UI thread. Cancelling writes nothing.
    pub fn export(&mut self, cx: &mut Context<Self>) {
        let events: Vec<_> = self.model(cx).events().cloned().collect();
        if events.is_empty() {
            self.toast(Toast::info("There are no events to export."), cx);
            return;
        }
        let context = self
            .deps
            .sessions
            .get(&self.cluster)
            .map_or_else(|| self.cluster.to_string(), |s| s.context().to_string());
        let name = suggested_file_name(&context, self.deps.clock.now());
        let directory = home_directory();
        let answer = cx.prompt_for_new_path(&directory, Some(&name));
        // Replaced (and so cancelled) by the next export, never cleared from inside.
        self.export_task = Some(cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = answer.await else {
                return;
            };
            this.update(cx, |view, cx| view.write_export(path, events, cx))
                .ok();
        }));
    }

    /// Writes `events` to `path`: serialised and written on the Tokio bridge.
    pub fn write_export(
        &mut self,
        path: PathBuf,
        events: Vec<std::sync::Arc<oxikube_domain::event::Event>>,
        cx: &mut Context<Self>,
    ) {
        let fs = self.deps.fs.clone();
        let count = events.len();
        let target = path.clone();
        let work = spawn_kube(cx, async move {
            let bytes = export_json(&events);
            fs.write(&target, &bytes).await
        });
        self.export_task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |view, cx| {
                let toast = match result {
                    Ok(Ok(())) => {
                        Toast::success(format!("Exported {count} events to {}", path.display()))
                    }
                    Ok(Err(error)) => {
                        tracing::warn!(%error, "exporting the events failed");
                        Toast::error(format!("Could not export the events: {}", error.message()))
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the events export stopped");
                        Toast::error("The export stopped before it finished.")
                    }
                };
                view.toast(toast.key("events-export"), cx);
            })
            .ok();
        }));
    }

    /// Shows `toast` in the workspace the view is a tab of; nothing without one.
    pub(super) fn toast(&self, toast: Toast, cx: &mut Context<Self>) {
        if let Some(workspace) = &self.workspace {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.show_toast(toast, cx);
                })
                .ok();
        }
    }

    /// Puts the focus in the text filter (`/`).
    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fields
            .text
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Gives the focus back to the rows.
    pub fn focus_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }
}

/// Where the save panel opens: the user's home.
fn home_directory() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}
