//! The find's operations on a [`DetailView`]: opening, typing, scanning, stepping.

use std::ops::Range;
use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, Window};
use oxikube_app::search::find::{FindMatches, FindQuery, find_in_text};
use oxikube_domain::command::Command;
use oxikube_ui::code_view::CodeView;
use oxikube_ui::input::{InputEvent, InputState};

use crate::detail::tabs::DetailTab;
use crate::detail::view::DetailView;

impl DetailView {
    /// The code view of the tab shown, when it is one a find can search (YAML, Describe).
    fn find_view(&self) -> Option<Entity<CodeView>> {
        match self.tab {
            DetailTab::Yaml => self.yaml.view.clone(),
            DetailTab::Describe => self.describe.view.clone(),
            _ => None,
        }
    }

    /// The text of the tab shown, as the code view was given it.
    fn find_text(&self) -> Option<Arc<str>> {
        match self.tab {
            DetailTab::Yaml => self.yaml.text.as_ref()?.result.as_ref().ok().cloned(),
            DetailTab::Describe => self.describe.output.as_ref().map(|o| o.text.clone()),
            _ => None,
        }
    }

    /// Whether the find field has the keyboard focus, read from the window.
    pub(in crate::detail) fn find_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.find.input.as_ref().is_some_and(|input| {
            gpui::Focusable::focus_handle(input.read(cx), cx).contains_focused(window, cx)
        })
    }

    /// Whether the find strip is open.
    pub fn find_open(&self) -> bool {
        self.find.open
    }

    /// The pattern in the field.
    pub fn find_text_typed(&self) -> &str {
        &self.find.text
    }

    /// How many matches the pattern has in the text shown.
    pub fn find_match_count(&self) -> usize {
        self.find.nav.len()
    }

    /// The match the user is on, counted from 1, and how many there are.
    pub fn find_position(&self) -> Option<(usize, usize)> {
        self.find.nav.position()
    }

    /// Why the pattern in the field is not one.
    pub fn find_error(&self) -> Option<&oxikube_app::search::filter::FilterError> {
        self.find.error.as_ref()
    }

    /// The label next to the field: `3 / 12`, `No matches`, or `10,000+` when the scan stopped
    /// at its cap. `None` without a pattern.
    pub fn find_label(&self) -> Option<String> {
        self.find.query.as_ref()?;
        let nav = &self.find.nav;
        let more = if nav.truncated() { "+" } else { "" };
        Some(match nav.position() {
            _ if nav.is_empty() => "No matches".to_owned(),
            Some((at, total)) => format!("{at} / {total}{more}"),
            None => format!("{}{more}", nav.len()),
        })
    }

    /// Opens the strip and puts the keyboard in the field; with `pattern`, searches for it. On
    /// a tab with no text to search (Overview, Events), the YAML tab is shown first.
    pub fn find(&mut self, pattern: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.tab, DetailTab::Yaml | DetailTab::Describe) {
            self.set_tab(DetailTab::Yaml, cx);
        }
        self.find.open = true;
        self.ensure_find_input(window, cx);
        if let Some(pattern) = pattern {
            self.set_find_text(pattern, window, cx);
        }
        if let Some(input) = &self.find.input {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn ensure_find_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.input.is_some() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in the text"));
        self.find.subscription = Some(cx.subscribe_in(&input, window, Self::on_find_input));
        self.find.input = Some(input);
    }

    fn on_find_input(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let text = input.read(cx).value().to_string();
                self.edit_find(&text, cx);
            }
            // Enter: keep the matches, give the keyboard back so `n` / `N` step through them.
            InputEvent::PressEnter { .. } => {
                if self.find.nav.current().is_none() {
                    self.next_match(cx);
                }
                window.focus(&self.focus, cx);
            }
            InputEvent::Focus | InputEvent::Blur => cx.notify(),
        }
    }

    /// Puts `text` in the field and searches for it, as if typed.
    pub fn set_find_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_find_input(window, cx);
        if let Some(input) = &self.find.input {
            input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        }
        self.edit_find(text, cx);
    }

    /// The text in the field changed. A pattern that compiles is searched for at once (the first
    /// match at or after the top of the screen becomes current); one that does not keeps the
    /// matches of the last good pattern and says why.
    pub(in crate::detail) fn edit_find(&mut self, text: &str, cx: &mut Context<Self>) {
        if text == self.find.text && self.find.error.is_none() {
            return;
        }
        text.clone_into(&mut self.find.text);
        match FindQuery::new(text) {
            Ok(query) => {
                self.find.error = None;
                self.find.query = query;
                self.rescan_find(true, cx);
            }
            Err(error) => {
                self.find.error = Some(error);
                cx.notify();
            }
        }
    }

    /// Closes the strip, forgets the matches and returns the keyboard to the detail (`escape` in
    /// the field).
    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find.open = false;
        self.find.query = None;
        self.find.error = None;
        self.find.text.clear();
        self.find.task = None;
        self.find.generation += 1;
        self.find.clear_matches();
        if let Some(input) = &self.find.input {
            input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
        }
        self.clear_code_view_matches(cx);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Takes the match colouring off the YAML and Describe code views.
    fn clear_code_view_matches(&self, cx: &mut Context<Self>) {
        for view in [&self.yaml.view, &self.describe.view].into_iter().flatten() {
            view.update(cx, |view, cx| view.clear_matches(cx));
        }
    }

    /// Searches the text of the tab shown again: the text changed (a new object version, a tab
    /// switch) or the pattern did. With `select`, the first match at or after the top of the
    /// screen becomes the current one and is scrolled to.
    pub(in crate::detail) fn rescan_find(&mut self, select: bool, cx: &mut Context<Self>) {
        self.find.generation += 1;
        let generation = self.find.generation;
        let (Some(view), Some(text), Some(query)) =
            (self.find_view(), self.find_text(), self.find.query.clone())
        else {
            self.find.task = None;
            self.find.clear_matches();
            self.clear_code_view_matches(cx);
            cx.notify();
            return;
        };
        let anchor = view.read(cx).top_byte() as u64;
        let scanned = text.clone();
        self.find.task = Some(cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move { find_in_text(&scanned, &query) })
                .await;
            this.update(cx, |view, cx| {
                view.found(generation, text, found, select.then_some(anchor), cx);
            })
            .ok();
        }));
        cx.notify();
    }

    /// Applies a scan, unless a newer one replaced the one it answers.
    fn found(
        &mut self,
        generation: u64,
        text: Arc<str>,
        found: FindMatches,
        select_from: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.find.generation
            || self.find_text().is_none_or(|t| !Arc::ptr_eq(&t, &text))
        {
            return;
        }
        let starts = found.ranges.iter().map(|r| r.start as u64).collect();
        // An edit starts the search over: the match the last pattern had current is not kept
        // (`set` would keep it and `next` would then step past it).
        if select_from.is_some() {
            self.find.nav.clear();
        }
        self.find.nav.set(starts, found.truncated);
        self.find.ranges = found.ranges.into();
        if let Some(anchor) = select_from {
            self.find.nav.next(anchor);
        }
        self.show_matches(&text, select_from.is_some(), cx);
        cx.notify();
    }

    /// Hands the matches to the code view, and with `reveal` scrolls to the current one.
    fn show_matches(&mut self, text: &Arc<str>, reveal: bool, cx: &mut Context<Self>) {
        let Some(view) = self.find_view() else {
            return;
        };
        let ranges: Arc<[Range<usize>]> = self.find.ranges.clone();
        let current = self.find.nav.current_index();
        let to = current.and_then(|i| ranges.get(i)).map(|r| r.start);
        let text = text.clone();
        view.update(cx, |view, cx| {
            view.set_matches(text, ranges, current, cx);
            if reveal && let Some(at) = to {
                view.scroll_to_byte(at, cx);
            }
        });
    }

    /// Goes to the next match, wrapping from the last to the first (`resource::NextMatch`).
    pub fn next_match(&mut self, cx: &mut Context<Self>) {
        self.step_match(true, cx);
    }

    /// Goes to the previous match, wrapping from the first to the last
    /// (`resource::PreviousMatch`).
    pub fn previous_match(&mut self, cx: &mut Context<Self>) {
        self.step_match(false, cx);
    }

    fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        let (Some(view), Some(text)) = (self.find_view(), self.find_text()) else {
            return;
        };
        let moved = if forward {
            self.find.nav.next(view.read(cx).top_byte() as u64)
        } else {
            self.find.nav.previous()
        };
        if moved.is_some() {
            self.show_matches(&text, true, cx);
            cx.notify();
        }
    }

    /// `/`: opens the find at once (so the keys typed right after it are text) and sends
    /// `resource::Find`.
    pub(in crate::detail) fn request_find(
        &mut self,
        pattern: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find(pattern.as_deref(), window, cx);
        self.find.echoes = self.find.echoes.saturating_add(1);
        let command = Command::ResourceFind {
            target: self.target.clone(),
            pattern,
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// The `resource::Find` command reached this view: opens the find, unless the command is the
    /// echo of a `/` pressed here, which opened it already.
    pub fn find_on_command(
        &mut self,
        pattern: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.find.echoes > 0 && pattern.is_none() {
            self.find.echoes -= 1;
            return;
        }
        self.find(pattern, window, cx);
    }

    /// `n`: sends `resource::NextMatch`.
    pub fn request_next_match(&mut self, cx: &mut Context<Self>) {
        let command = Command::ResourceNextMatch {
            target: self.target.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// `N`: sends `resource::PreviousMatch`.
    pub fn request_previous_match(&mut self, cx: &mut Context<Self>) {
        let command = Command::ResourcePreviousMatch {
            target: self.target.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }
}
