//! What keys, clicks and the search field do to the catalog.

use gpui::{Context, ScrollStrategy, Window};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;

use super::CatalogView;

impl CatalogView {
    /// Moves the keyboard focus to the search field.
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |search, cx| search.focus(window, cx));
    }

    /// Puts `text` in the search field and filters by it, as if it was typed.
    pub fn set_search(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |search, cx| {
            search.set_value(text.to_owned(), window, cx)
        });
        self.on_query_changed(cx);
    }

    /// The search text changed: refilter, select the best match and show it first.
    pub(super) fn on_query_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value();
        if self.model.set_query(&query) {
            if self.model.is_searching() {
                self.scroll.scroll_to_item(0, ScrollStrategy::Top);
            } else if let Some(ix) = self.model.selected_index() {
                // Cleared: the selection stays, so keep it in view.
                self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            }
        }
        cx.notify();
    }

    /// Moves the selection by `delta` rows and keeps it in view.
    pub fn select_by(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.model.select_by(delta) {
            self.reveal_selection(cx);
        }
    }

    /// Selects the first row.
    pub fn select_first(&mut self, cx: &mut Context<Self>) {
        if self.model.select_first() {
            self.reveal_selection(cx);
        }
    }

    /// Selects the last row.
    pub fn select_last(&mut self, cx: &mut Context<Self>) {
        if self.model.select_last() {
            self.reveal_selection(cx);
        }
    }

    fn reveal_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(ix) = self.model.selected_index() {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    /// Selects the visible row `ix` and connects it: what a click does.
    pub fn activate_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.model.select(ix);
        self.connect_selected(cx);
    }

    /// Sends `cluster::Connect` for the selected cluster. Nothing is selected when the list is
    /// empty, and then nothing is sent.
    pub fn connect_selected(&mut self, cx: &mut Context<Self>) {
        let Some(cluster) = self.model.selected().cloned() else {
            return;
        };
        self.connect(cluster, cx);
    }

    fn connect(&mut self, cluster: ClusterId, cx: &mut Context<Self>) {
        // Shown at once; the order is not touched, so a row does not jump under the pointer.
        // The state db and the next read catch up.
        let now = self.deps.clock.now();
        self.model.set_last_used(&cluster, now);
        self.deps
            .dispatcher
            .dispatch(Command::ClusterConnect { cluster }, cx);
        cx.notify();
    }

    /// Sends `cluster::Disconnect` for the selected cluster.
    pub fn disconnect_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(cluster) = self.model.selected().cloned() {
            self.deps
                .dispatcher
                .dispatch(Command::ClusterDisconnect { cluster }, cx);
        }
    }

    /// Flips the favourite flag of the selected cluster.
    pub fn toggle_favourite_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(cluster) = self.model.selected().cloned() {
            self.toggle_favourite(cluster, cx);
        }
    }

    /// Flips the favourite flag of `cluster`: the star moves at once and `cluster::ToggleFavourite`
    /// persists it. The command carries the new value, not "flip", so running it twice is safe.
    pub(super) fn toggle_favourite(&mut self, cluster: ClusterId, cx: &mut Context<Self>) {
        let Some(favourite) = self
            .model
            .visible_rows()
            .find(|row| row.entry().id() == &cluster)
            .map(|row| !row.entry().favourite)
        else {
            return;
        };
        self.model.set_favourite(&cluster, favourite);
        self.deps.dispatcher.dispatch(
            Command::ClusterToggleFavourite {
                cluster,
                favourite: Some(favourite),
            },
            cx,
        );
        self.reveal_selection(cx);
    }
}
