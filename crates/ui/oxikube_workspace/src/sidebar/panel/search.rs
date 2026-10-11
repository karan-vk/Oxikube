//! The Custom Resources search field (E12-S17): typing filters the section's groups and kinds,
//! Enter opens the first kind that matches.
//!
//! The field is a text input drawn in the [`SearchRow`](crate::sidebar::SearchRow) of the list.
//! It is built on the first draw that shows it (an input needs a window) and lives as long as the
//! panel. Each edit rebuilds the rows once (a filter over the kinds discovery found, no cluster
//! call, no command: it is navigation state), so it keeps up with typing with hundreds of CRDs.

use gpui::{AppContext as _, Context, Entity, Subscription, Window};
use oxikube_ui::input::{InputEvent, InputState};

use super::SidebarPanel;
use crate::sidebar::rows::Row;

/// The input and the subscription that follows its edits.
pub(super) struct SearchField {
    pub(super) input: Entity<InputState>,
    _subscription: Subscription,
}

impl SidebarPanel {
    /// What the Custom Resources search field holds.
    pub fn custom_query(&self) -> &str {
        &self.custom_query
    }

    /// Filters Custom Resources by `query` (see [`SearchRow`](crate::sidebar::SearchRow)): the
    /// field's edits land here. Redraws only when the rows changed.
    pub fn set_custom_query(&mut self, query: &str, cx: &mut Context<Self>) {
        if self.custom_query != query {
            self.custom_query = query.to_owned();
            self.rebuild(cx);
        }
    }

    /// Opens the first custom kind the search left on screen, as a click on it would. `false`
    /// when none is shown.
    pub fn open_first_custom_match(&mut self, cx: &mut Context<Self>) -> bool {
        let first = self.rows.iter().find_map(|row| match row {
            Row::Entry(entry) if entry.id.starts_with("crd:") => Some(entry.id.to_string()),
            _ => None,
        });
        match first {
            Some(id) => {
                self.activate(&id, cx);
                true
            }
            None => false,
        }
    }

    /// The search field's input, once a draw built it.
    #[cfg(test)]
    pub(crate) fn search_field(&self) -> Option<Entity<InputState>> {
        self.custom_search.as_ref().map(|field| field.input.clone())
    }

    /// The search field, built the first time a draw needs it.
    pub(super) fn search_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(field) = &self.custom_search {
            return field.input.clone();
        }
        let query = self.custom_query.clone();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Search custom resources");
            if !query.is_empty() {
                state.set_value(query, window, cx);
            }
            state
        });
        let subscription = cx.subscribe_in(&input, window, Self::on_search_event);
        self.custom_search = Some(SearchField {
            input: input.clone(),
            _subscription: subscription,
        });
        input
    }

    fn on_search_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let text = input.read(cx).value().to_string();
                self.set_custom_query(&text, cx);
            }
            InputEvent::PressEnter { .. } => {
                self.open_first_custom_match(cx);
            }
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }
}
