//! The table's side of the filter bar (E07-S04): applying a filter to the subscription, focusing
//! the bar, and saving and restoring its text.
//!
//! The bar parses and debounces ([`FilterBar`](crate::filter::FilterBar)); here a good filter
//! becomes [`Subscription::set_filter_parts`]: the text, inverse and fuzzy parts are applied by
//! the store to its cache (incrementally, off the UI thread), a label selector re-keys the
//! subscription's feeds so the server filters. The scope is never touched, so the filter composes
//! with the session's namespace selection.

use gpui::{App, AppContext as _, Context, Entity, Focusable as _, Window};
use oxikube_app::store::FilterParts;
use oxikube_domain::command::Command;
use oxikube_settings::Settings as _;

use super::view::ResourceTable;
use crate::filter::{
    ClearFilter, FilterBar, FilterBarEvent, FilterWriter, FocusFilter, ResourceTableSettings,
    SavedFilter,
};

impl ResourceTable {
    pub(super) fn on_filter_event(
        &mut self,
        _: &Entity<FilterBar>,
        event: &FilterBarEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            FilterBarEvent::Changed { parts, text } => {
                self.apply_filter(parts.clone(), text, cx);
                self.save_filter(text, cx);
            }
            FilterBarEvent::Returned => window.focus(&self.focus, cx),
            // The chip's cross: the command the jump bar and an agent use to set (here: remove)
            // the filter; it comes back through the bus and clears the bar.
            FilterBarEvent::ClearRequested => {
                let command = Command::TableSetFilter {
                    cluster: self.cluster.clone(),
                    gvk: self.kind.gvk.clone(),
                    text: String::new(),
                };
                self.deps.dispatcher.dispatch(command, cx);
            }
            // The flag itself is read from the window when the table renders (`filter_focused`).
            FilterBarEvent::Editing(_) => cx.notify(),
        }
    }

    /// Runs the subscription with `parts`: the filter, the selector the server applies, and the
    /// sort that goes with them (a fuzzy filter ranks unless a column is chosen).
    pub(super) fn apply_filter(&mut self, parts: FilterParts, text: &str, cx: &mut Context<Self>) {
        self.filter_parts = parts.clone();
        // How the filter reads in the "no matches" state.
        let label = (!parts.is_empty()).then(|| text.to_owned());
        self.table.update_quiet(cx, |d| d.filter = label);
        let sort = self.sort_key(cx);
        if let Some(subscription) = &mut self.subscription {
            subscription.set_filter_parts(parts, sort);
        }
    }

    /// The filter the table shows rows for.
    pub fn filter_parts(&self) -> &FilterParts {
        &self.filter_parts
    }

    /// Moves the focus to the filter bar (what `table::FocusFilter` does).
    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |bar, cx| bar.focus(window, cx));
    }

    /// The `table::FocusFilter` command reached this table: focuses the bar, unless the command
    /// is the echo of a `/` pressed here, which focused the bar already (and the user may have
    /// left it again since, with `enter`, before the command came back).
    pub fn focus_filter_on_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter_focus_echoes > 0 {
            self.filter_focus_echoes -= 1;
            return;
        }
        self.focus_filter(window, cx);
    }

    /// Whether the filter field has the keyboard focus, read from the window. The key context is
    /// built from it on every render, and GPUI draws a dirty frame before it dispatches a key, so
    /// the key right after the focus moved already sees `Editing`. (The bar's focus events come a
    /// frame later, and not at all while the window is inactive: a key typed in that gap would
    /// run a table action instead of typing.)
    pub(super) fn filter_focused(&self, window: &Window, cx: &App) -> bool {
        self.filter
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
    }

    /// Types `text` into the bar and applies it, as a restored filter or a test does.
    pub fn set_filter_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.filter
            .update(cx, |bar, cx| bar.set_text(text, window, cx));
    }

    /// `/`: focuses the filter bar at once and sends `table::FocusFilter`, so the key, the
    /// palette and an agent run one command. The focus does not wait for the command's round
    /// trip through the bus: the keys typed right after `/` are filter text, never table actions
    /// (`/apple` typed fast must not attach to a pod on its `a`).
    pub(super) fn on_focus_filter(
        &mut self,
        _: &FocusFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_filter(window, cx);
        self.filter_focus_echoes = self.filter_focus_echoes.saturating_add(1);
        let command = Command::TableFocusFilter {
            cluster: self.cluster.clone(),
            gvk: self.kind.gvk.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// `escape` in the bar: clears the filter and returns the focus to the rows.
    pub(super) fn on_clear_filter(
        &mut self,
        _: &ClearFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.filter.update(cx, |bar, cx| bar.clear(window, cx));
    }

    /// Whether filters are saved (`resource_table.persist_filter`, read when it is needed so a
    /// settings change applies at once).
    fn persists_filter(cx: &gpui::App) -> bool {
        ResourceTableSettings::try_get(cx).is_some_and(|s| s.persist_filter)
    }

    /// Reads the saved filter in the background and puts it in the bar, when the setting is on.
    pub(super) fn load_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !Self::persists_filter(cx) {
            return;
        }
        let Ok(saved) = SavedFilter::new(self.deps.state.clone(), &self.cluster, &self.kind.gvk)
        else {
            return;
        };
        let load = cx.background_spawn(async move { saved.load().await });
        self.filter_task = Some(cx.spawn_in(window, async move |this, cx| {
            let text = match load.await {
                Ok(text) => text.unwrap_or_default(),
                Err(error) => {
                    tracing::warn!(%error, "reading the saved filter failed: no filter");
                    String::new()
                }
            };
            if text.is_empty() {
                return;
            }
            this.update_in(cx, |view, window, cx| {
                // Typed meanwhile: the user's text wins over the saved one.
                if view.filter.read(cx).text().is_empty() {
                    view.set_filter_text(&text, window, cx);
                }
            })
            .ok();
        }));
    }

    /// Saves `text` as this kind's filter, when the setting is on.
    fn save_filter(&mut self, text: &str, cx: &mut Context<Self>) {
        if !Self::persists_filter(cx) {
            return;
        }
        if self.filter_writer.is_none() {
            let Ok(saved) =
                SavedFilter::new(self.deps.state.clone(), &self.cluster, &self.kind.gvk)
            else {
                return;
            };
            self.filter_writer = Some(FilterWriter::spawn(saved, cx));
        }
        if let Some(writer) = &self.filter_writer {
            writer.save(text.to_owned());
        }
    }
}
