//! The search operations the `logs::*` commands call on a view, and the requests that ask for
//! them.
//!
//! A key, a bar button, the palette and an agent all go through the bus: `request_*` sends the
//! command with the view's target, the bus hands it to the window's
//! [`LogViews`](crate::LogViews), which calls the operation (`find`, `next_match`, ...). Typing in
//! the field is the one thing that does not: it is text editing, and applies at once. Nothing
//! here changes a cluster, so no `MutationGuard` tier applies.

use gpui::{Context, Window};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;

use super::state::{Edit, SavedSearch, SearchState};
use super::{Counts, SearchMemory};
use crate::LogView;

impl LogView {
    /// Opens the search bar and puts the keyboard in it; with `pattern`, searches for it.
    pub fn find(&mut self, pattern: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        self.search.state.set_open(true);
        self.ensure_search_input(window, cx);
        if let Some(pattern) = pattern {
            self.set_search_text(pattern, window, cx);
        }
        if let Some(input) = &self.search.input {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        self.save_search();
        cx.notify();
    }

    /// Searches for `text`, as if typed.
    pub fn set_search_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = &self.search.input {
            input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        }
        self.edit_search(text, cx);
    }

    /// The text in the bar changed.
    pub(crate) fn edit_search(&mut self, text: &str, cx: &mut Context<Self>) {
        let edit = self.search.state.set_text(text);
        self.apply_edit(edit, cx);
    }

    fn apply_edit(&mut self, edit: Edit, cx: &mut Context<Self>) {
        match edit {
            Edit::Rematch => self.rematch(cx),
            Edit::Remode => self.remode(cx),
            Edit::Unchanged => cx.notify(),
        }
        self.save_search();
    }

    /// Goes to the next match (from the top of the screen when none is current), wrapping from
    /// the last match to the first.
    pub fn next_match(&mut self, cx: &mut Context<Self>) {
        self.step_match(true, cx);
    }

    /// Goes to the previous match, wrapping from the first to the last.
    pub fn previous_match(&mut self, cx: &mut Context<Self>) {
        self.step_match(false, cx);
    }

    fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        let current = self.search.state.current();
        let anchor = self.top_seq().unwrap_or(0);
        let target = self.window.index().and_then(|index| {
            if forward {
                index.next(current, anchor)
            } else {
                index.prev(current)
            }
        });
        let Some(seq) = target else {
            cx.notify();
            return;
        };
        self.search.state.set_current(Some(seq));
        // Looking at a match is looking away from the newest line.
        self.pause(cx);
        self.reveal_seq(seq);
        cx.notify();
    }

    /// Makes the search case-sensitive, or not.
    pub fn toggle_case(&mut self, cx: &mut Context<Self>) {
        let edit = self.search.state.toggle_case();
        self.apply_edit(edit, cx);
    }

    /// Matches the lines without the pattern, or those with it.
    pub fn toggle_inverse(&mut self, cx: &mut Context<Self>) {
        let edit = self.search.state.toggle_inverse();
        self.apply_edit(edit, cx);
    }

    /// Hides the lines that do not match, or shows them all again with the matches highlighted.
    pub fn toggle_filter_mode(&mut self, cx: &mut Context<Self>) {
        let edit = self.search.state.toggle_mode();
        self.apply_edit(edit, cx);
    }

    /// Closes the bar and clears the highlights and the filter; the keyboard goes back to the
    /// log.
    pub fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.search.state.is_open() {
            return;
        }
        self.search.state.set_open(false);
        if let Some(input) = &self.search.input {
            input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
        }
        self.search.editing = false;
        self.rematch(cx);
        self.save_search();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Keeps the search for the session (or forgets it once the bar is closed).
    pub(crate) fn save_search(&self) {
        let Some(memory) = &self.search.memory else {
            return;
        };
        if self.search.state.is_open() {
            memory.save(&self.target, self.search.state.saved());
        } else {
            memory.forget(&self.target);
        }
    }

    /// Where the search is kept for the session; the controller gives every view of its window
    /// the same memory.
    pub fn set_search_memory(&mut self, memory: SearchMemory) {
        self.search.memory = Some(memory);
    }

    /// Puts back a search the session remembered for this target.
    pub fn restore_search(&mut self, saved: &SavedSearch, cx: &mut Context<Self>) {
        self.search.state = SearchState::restored(saved);
        self.rematch(cx);
        cx.notify();
    }

    /// The state of the search.
    pub fn search_state(&self) -> &SearchState {
        &self.search.state
    }

    /// The numbers behind the status words.
    pub fn search_counts(&self) -> Counts {
        let index = self.window.index();
        Counts {
            matches: index.map_or(0, |index| index.len()),
            ordinal: self
                .search
                .state
                .current()
                .and_then(|seq| index?.position(seq))
                .map(|position| position + 1),
            lines: self.window.retained_count(),
            scanning: self.search.scanning,
        }
    }

    /// Whether the keyboard is in the search field.
    pub fn is_searching(&self) -> bool {
        self.search.editing
    }

    /// Sends the command `make` builds for this view's target.
    fn request(&mut self, make: impl FnOnce(ResourceRef) -> Command, cx: &mut Context<Self>) {
        let command = make(self.target.clone());
        self.send(command, cx);
    }

    /// Asks to open the search bar (`logs::Find`).
    pub fn request_find(&mut self, cx: &mut Context<Self>) {
        self.request(
            |target| Command::LogsFind {
                target,
                pattern: None,
            },
            cx,
        );
    }

    /// Asks for the next match (`logs::NextMatch`).
    pub fn request_next_match(&mut self, cx: &mut Context<Self>) {
        self.request(|target| Command::LogsNextMatch { target }, cx);
    }

    /// Asks for the previous match (`logs::PreviousMatch`).
    pub fn request_previous_match(&mut self, cx: &mut Context<Self>) {
        self.request(|target| Command::LogsPreviousMatch { target }, cx);
    }

    /// Asks to toggle case sensitivity (`logs::ToggleCase`).
    pub fn request_toggle_case(&mut self, cx: &mut Context<Self>) {
        self.request(|target| Command::LogsToggleCase { target }, cx);
    }

    /// Asks to toggle the inverse match (`logs::ToggleInverse`).
    pub fn request_toggle_inverse(&mut self, cx: &mut Context<Self>) {
        self.request(|target| Command::LogsToggleInverse { target }, cx);
    }

    /// Asks to toggle between highlighting and filtering (`logs::ToggleFilterMode`).
    pub fn request_toggle_filter_mode(&mut self, cx: &mut Context<Self>) {
        self.request(|target| Command::LogsToggleFilterMode { target }, cx);
    }

    /// Asks to close the search bar (`logs::CloseSearch`).
    pub fn request_close_search(&mut self, cx: &mut Context<Self>) {
        self.request(|target| Command::LogsCloseSearch { target }, cx);
    }
}
