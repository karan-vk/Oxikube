//! Opening and closing groups, the highlight, and activating rows.

use gpui::{AppContext as _, Context, ScrollStrategy, SharedString};

use super::{SidebarEvent, SidebarPanel};
use crate::sidebar::rows::Row;

impl SidebarPanel {
    /// Whether the row `id` is an open section or group.
    pub fn is_open(&self, id: &str) -> bool {
        match self.row(id) {
            Some(Row::Section(section)) => section.open,
            Some(Row::Group(group)) => group.open,
            _ => false,
        }
    }

    /// Opens or closes the section or group `id` and remembers the choice for this cluster.
    /// Returns `false` when there is no such expandable row.
    pub fn set_open(&mut self, id: &str, open: bool, cx: &mut Context<Self>) -> bool {
        let expandable = match self.row(id) {
            Some(Row::Section(section)) => section.expandable,
            Some(Row::Group(_)) => true,
            _ => false,
        };
        if !expandable {
            return false;
        }
        if self.is_open(id) != open {
            self.open.insert(id.to_owned(), open);
            self.rebuild(cx);
            self.save(cx);
        }
        true
    }

    /// Flips the section or group `id`.
    pub fn toggle(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let open = self.is_open(id);
        self.set_open(id, !open, cx)
    }

    /// What a click or Enter on the row `id` does: a section or group opens or closes, an entry
    /// (or a heading with nothing to open) navigates.
    pub fn activate(&mut self, id: &str, cx: &mut Context<Self>) {
        let target = match self.row(id) {
            Some(Row::Section(section)) if section.expandable => {
                self.toggle(id, cx);
                return;
            }
            Some(Row::Section(section)) => section.target.clone(),
            Some(Row::Group(_)) => {
                self.toggle(id, cx);
                return;
            }
            Some(Row::Entry(entry)) => entry.target.clone(),
            _ => return,
        };
        self.highlighted = Some(id.to_owned().into());
        if let Some(target) = target {
            self.selected = Some(id.to_owned().into());
            cx.emit(SidebarEvent::Navigate(target));
        }
        cx.notify();
    }

    /// Moves the highlight by `delta` interactive rows, stopping at the ends.
    pub fn move_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.is_interactive())
            .map(|(ix, _)| ix)
            .collect();
        if rows.is_empty() {
            return;
        }
        let current = self
            .highlighted
            .as_deref()
            .and_then(|id| rows.iter().position(|ix| self.rows[*ix].id() == id));
        let next = match current {
            None if delta >= 0 => 0,
            None => rows.len() - 1,
            Some(at) => at.saturating_add_signed(delta).min(rows.len() - 1),
        };
        let ix = rows[next];
        self.highlighted = Some(self.rows[ix].id().to_owned().into());
        self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Highlights the row `id` (the pointer is over it).
    pub(super) fn hover(&mut self, id: SharedString, cx: &mut Context<Self>) {
        if self.highlighted.as_ref() != Some(&id) {
            self.highlighted = Some(id);
            cx.notify();
        }
    }

    /// Keeps the highlight on a row that still exists.
    pub(super) fn fix_highlight(&mut self) {
        let still_there = self
            .highlighted
            .as_deref()
            .is_some_and(|id| self.rows.iter().any(|row| row.id() == id));
        if !still_there {
            self.highlighted = None;
        }
    }

    /// Writes the user's open and closed choices, off the UI thread. The write is detached: it
    /// finishes even when the cluster's tab closes right after the click.
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let open = self.open.clone();
        cx.background_spawn(async move {
            if let Err(error) = store.save(&open).await {
                tracing::warn!(%error, "saving the sidebar state failed");
            }
        })
        .detach();
    }
}
