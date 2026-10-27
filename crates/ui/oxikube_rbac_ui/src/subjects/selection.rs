//! Selecting rows and opening a subject.

use gpui::{Context, Window};
use oxikube_app::rbac_subjects::subject_name;
use oxikube_domain::command::Command;
use oxikube_ui::table::{RowClick, TableEvent};

use super::model::Page;
use super::view::SubjectsView;

impl SubjectsView {
    pub(super) fn on_list_event(&mut self, event: &TableEvent, cx: &mut Context<Self>) {
        if let TableEvent::RowClicked(RowClick { row, count, .. }) = event {
            self.select_row(*row, cx);
            if *count >= 2 {
                self.open_selected(cx);
            }
        }
    }

    pub(super) fn on_grid_event(&mut self, event: &TableEvent, cx: &mut Context<Self>) {
        if let TableEvent::RowClicked(RowClick { row, .. }) = event {
            self.select_row(*row, cx);
        }
    }

    /// Selects row `row` of the shown table.
    pub fn select_row(&mut self, row: usize, cx: &mut Context<Self>) {
        if self.page == Page::Subject {
            self.grid.update_quiet(cx, |d| d.select_row(Some(row)));
            self.grid.refresh(cx);
            self.grid.reveal_row(row, cx);
            self.read_why(cx);
        } else {
            let Some(subject) = self.list.read(cx, |d| d.subject(row).cloned()) else {
                return;
            };
            self.list.update_quiet(cx, |d| d.select(Some(subject)));
            self.list.refresh(cx);
            self.list.reveal_row(row, cx);
        }
        cx.notify();
    }

    /// Moves the selection `delta` rows (the first row when none is selected).
    pub fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let (count, current) = if self.page == Page::Subject {
            self.grid.read(cx, |d| (d.len(), d.selected_row()))
        } else {
            self.list.read(cx, |d| {
                (d.len(), d.selected().and_then(|s| d.position_of(s)))
            })
        };
        if count == 0 {
            return;
        }
        let next = match current {
            None => 0,
            Some(row) => row.saturating_add_signed(delta).min(count - 1),
        };
        self.select_row(next, cx);
    }

    /// Selects the first or the last row.
    pub fn select_end(&mut self, last: bool, cx: &mut Context<Self>) {
        let count = if self.page == Page::Subject {
            self.grid.read(cx, |d| d.len())
        } else {
            self.list.read(cx, |d| d.len())
        };
        if count > 0 {
            self.select_row(if last { count - 1 } else { 0 }, cx);
        }
    }

    /// Opens the matrix of the selected list row: the command `rbac::ShowSubject`, the same one
    /// `:rbac`, the palette and an agent run.
    pub fn open_selected(&mut self, cx: &mut Context<Self>) {
        if self.page == Page::Subject {
            return;
        }
        let Some(subject) = self.list.read(cx, |d| d.selected().cloned()) else {
            return;
        };
        self.deps.dispatcher.dispatch(
            Command::RbacShowSubject {
                cluster: self.cluster.clone(),
                subject: subject_name(&subject).to_string(),
            },
            cx,
        );
    }

    pub(super) fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn focus_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.set_editing(false, cx);
    }
}
