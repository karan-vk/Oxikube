//! [`CommandPaletteDelegate`]: the command palette as a [`PickerDelegate`].
//!
//! Holds the snapshot of the commands taken when the palette opened ([`Snapshot`]), the target
//! they run on ([`CommandTarget`]) and the matches of the current query. Nothing here calls a
//! service: confirming turns the selected id into `Command`s with [`commands_for`] and hands them
//! to the [`CommandDispatcher`] every other surface uses, so the guard, the audit log and the tool
//! stubs apply as for a key or a button (non-negotiable 4); a mutation asks its own confirmation
//! there, the palette never answers it. A command the focused view runs through its own flow (a
//! table's delete dialog) is left for that view instead ([`Launch::surface`]).

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, Context, DismissEvent, SharedString, Task, WeakEntity, Window};
use oxikube_app::{CommandTarget, InvokeError, RecentsStore, commands_for};
use oxikube_domain::command::CommandId;
use oxikube_workspace::{Toast, Workspace};

use super::outbox::{Launch, Outbox};
use super::rows::{Found, Row, Snapshot, into_found, ranks, recency_of};
use crate::picker::fuzzy::{self, INLINE_MATCH_LIMIT};
use crate::picker::{Picker, PickerDelegate};

/// What the palette is built from.
pub struct PaletteParts {
    /// The commands, classified for the context the palette opened in.
    pub snapshot: Snapshot,
    /// What the commands run on.
    pub target: CommandTarget,
    /// Where a confirmed command waits for the palette to close ([`Outbox`]).
    pub outbox: Outbox,
    /// The commands run lately.
    pub recents: Arc<dyn RecentsStore>,
    /// The workspace that shows the palette's toasts.
    pub workspace: WeakEntity<Workspace>,
}

/// The palette's picker delegate. See the [`command_palette`](crate::command_palette) module docs.
pub struct CommandPaletteDelegate {
    pub(super) snapshot: Snapshot,
    target: CommandTarget,
    outbox: Outbox,
    recents: Arc<dyn RecentsStore>,
    /// The recent commands when the palette opened, as a rank (0 is the latest).
    recent_rank: Arc<HashMap<CommandId, usize>>,
    workspace: WeakEntity<Workspace>,
    pub(super) show_all: bool,
    pub(super) found: Vec<Found>,
    selected: usize,
}

impl CommandPaletteDelegate {
    /// A delegate listing the available commands, recents first.
    pub fn new(parts: PaletteParts) -> Self {
        let recent_rank = Arc::new(ranks(&parts.recents.recent()));
        let mut delegate = Self {
            snapshot: parts.snapshot,
            target: parts.target,
            outbox: parts.outbox,
            recents: parts.recents,
            recent_rank,
            workspace: parts.workspace,
            show_all: false,
            found: Vec::new(),
            selected: 0,
        };
        // The empty query needs no matching: the first frame already lists the commands.
        delegate.found = delegate.match_now("");
        delegate
    }

    /// Whether the unavailable commands are listed too.
    pub fn show_all(&self) -> bool {
        self.show_all
    }

    /// The commands listed now, in order.
    pub fn listed(&self) -> Vec<CommandId> {
        self.found
            .iter()
            .map(|found| self.snapshot.rows[found.row].info.id())
            .collect()
    }

    /// The row of match `ix`.
    pub(super) fn row(&self, ix: usize) -> Option<(&Found, &Row)> {
        let found = self.found.get(ix)?;
        Some((found, &self.snapshot.rows[found.row]))
    }

    /// The selected command, whether or not it can run.
    pub fn selected_id(&self) -> Option<CommandId> {
        self.row(self.selected).map(|(_, row)| row.info.id())
    }

    /// Matches `query` on this thread (fine for the empty query and for short lists).
    fn match_now(&self, query: &str) -> Vec<Found> {
        let candidates = self.snapshot.candidates(self.show_all);
        let (rows, recent) = (&self.snapshot.rows, &self.recent_rank);
        into_found(fuzzy::match_strings_by(
            &candidates,
            query,
            usize::MAX,
            |row| recent.get(&rows[row].info.id()).copied(),
        ))
    }

    fn say(&self, toast: Toast, cx: &mut App) {
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| {
                workspace.show_toast(toast, cx);
            });
        }
    }
}

impl Picker<CommandPaletteDelegate> {
    /// Lists the unavailable commands too, or hides them again, and re-matches the query.
    pub(super) fn toggle_show_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.delegate.show_all = !self.delegate.show_all;
        self.refresh(window, cx);
        cx.notify();
    }
}

impl PickerDelegate for CommandPaletteDelegate {
    type ListItem = gpui::Stateful<gpui::Div>;

    fn match_count(&self) -> usize {
        self.found.len()
    }

    fn selected_index(&self) -> usize {
        self.selected
    }

    fn set_selected_index(&mut self, ix: usize, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.selected = ix;
    }

    fn placeholder_text(&self, _: &mut Window, _: &mut App) -> SharedString {
        "Type a command".into()
    }

    fn no_matches_text(&self, _: &mut Window, _: &mut App) -> Option<SharedString> {
        Some(if self.show_all || self.snapshot.hidden() == 0 {
            "No matching command".into()
        } else {
            "No matching command here. Show all lists the ones that cannot run now.".into()
        })
    }

    fn update_matches(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Task<()> {
        let candidates = self.snapshot.candidates(self.show_all);
        if query.trim().is_empty() || candidates.len() <= INLINE_MATCH_LIMIT {
            // Short enough to match inside this keystroke: shown in this frame.
            self.found = self.match_now(&query);
            self.selected = 0;
            return Task::ready(());
        }
        // A long list matches on the background executor; a newer keystroke drops this task.
        let recency = recency_of(self.snapshot.rows.clone(), self.recent_rank.clone());
        cx.spawn_in(window, async move |picker, cx| {
            let executor = cx.background_executor().clone();
            let matches =
                fuzzy::match_strings_async_by(candidates, query, usize::MAX, recency, &executor)
                    .await;
            let found = into_found(matches);
            picker
                .update(cx, |picker, _| {
                    picker.delegate.found = found;
                    picker.delegate.selected = 0;
                })
                .ok();
        })
    }

    fn confirm(&mut self, _secondary: bool, _: &mut Window, cx: &mut Context<Picker<Self>>) {
        let Some((_, row)) = self.row(self.selected) else {
            return;
        };
        // A command listed only for "show all" is shown with its reason: confirming does nothing.
        if row.unavailable.is_some() {
            return;
        }
        let (id, title) = (row.info.id(), row.info.title());
        if id == CommandId::PALETTE_TOGGLE {
            // "Toggle Command Palette" from the palette: it closes, and stays closed.
            cx.emit(DismissEvent);
            return;
        }
        if self.outbox.runs_on_surface(id) {
            // The view runs it through its own flow, which asks for what is missing: nothing is
            // built here (a select-all of a big table would stall the frame), and the bus
            // commands are made only if the view declines.
            self.recents.record(id);
            self.outbox
                .push(Launch::on_surface(id, self.target.clone()));
            cx.emit(DismissEvent);
            return;
        }
        match commands_for(id, &self.target) {
            Ok(commands) => {
                self.recents.record(id);
                // Sent by the host once the palette has closed and the focus is back, so the
                // command acts on the view the palette opened over.
                self.outbox.push(Launch::on_bus(id, commands));
            }
            Err(InvokeError::NeedsInput { .. }) => {
                // The palette is generic: a command that needs an operand has its own dialog.
                self.say(
                    Toast::warning(format!(
                        "{title} needs more information: use its button or menu."
                    ))
                    .key(format!("palette-input:{id}")),
                    cx,
                );
            }
            Err(error) => {
                tracing::warn!(%error, command = %id, "the palette cannot run a command");
                return;
            }
        }
        cx.emit(DismissEvent);
    }

    fn dismissed(&mut self, _: &mut Window, _: &mut Context<Picker<Self>>) {}

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<Self::ListItem> {
        self.render_row(ix, selected, cx)
    }

    fn render_footer(
        &self,
        _: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<gpui::AnyElement> {
        Some(self.render_footer(cx))
    }
}
