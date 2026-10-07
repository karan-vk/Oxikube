//! The catalog view model: entries, search, order, selection and connection states.
//!
//! Plain Rust, no GPUI: the render function reads it and the tests drive it directly, with
//! thousands of synthetic entries if they like. The view (`super::view`) owns one and applies
//! what happens (a load finished, a session changed, a key was pressed) to it.
//!
//! # Order
//!
//! Without a query the list is in [`CatalogEntry::cmp_default`] order: favourites, then the most
//! recently used, then by name. With a query it is the fuzzy-match order: best score first and,
//! for equal scores, the default order. Clearing the query restores the default order.
//!
//! # Selection
//!
//! The selection is a cluster id, not a row index, so it follows its cluster when a reload, a
//! favourite reorders the list, or a reload brings the entries again. When the selected cluster
//! leaves the visible list the first visible entry is selected instead. An empty list selects
//! nothing. Typing a search is the exception: it selects the best match (the first row), so
//! Enter connects the row at the top. Clearing the search keeps the selection.

mod matcher;
mod row;
mod status;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use gpui::SharedString;
use jiff::Timestamp;
use oxikube_app::CatalogEntry;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::ClusterSessionState;

use self::matcher::FuzzyMatcher;
pub use row::Row;
pub use status::{Badge, Tone};

/// Where the first read of the catalog stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    /// Nothing has come back yet.
    Loading,
    /// The entries are in (there may be none).
    Ready,
    /// The sources could not be read; the message is the source's own.
    Failed(SharedString),
}

/// The catalog as the view shows it. See the module docs.
pub struct CatalogModel {
    rows: Vec<Row>,
    visible: Vec<usize>,
    query: String,
    matcher: FuzzyMatcher,
    states: HashMap<ClusterId, ClusterSessionState>,
    selected: Option<ClusterId>,
    load: LoadState,
}

impl Default for CatalogModel {
    fn default() -> Self {
        Self::new()
    }
}

impl CatalogModel {
    /// An empty model, still loading.
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            visible: Vec::new(),
            query: String::new(),
            matcher: FuzzyMatcher::new(),
            states: HashMap::new(),
            selected: None,
            load: LoadState::Loading,
        }
    }

    /// Replaces the entries (a load finished) and applies the current query. Marks the catalog
    /// loaded.
    pub fn set_entries(&mut self, mut entries: Vec<CatalogEntry>) {
        entries.sort_by(CatalogEntry::cmp_default);
        self.rows = entries.into_iter().map(Row::new).collect();
        self.load = LoadState::Ready;
        self.refilter();
    }

    /// Records that the sources could not be read. The entries already shown stay.
    pub fn set_failed(&mut self, message: impl Into<SharedString>) {
        self.load = LoadState::Failed(message.into());
    }

    /// Where the load stands.
    pub fn load_state(&self) -> &LoadState {
        &self.load
    }

    /// Sets the search text and refilters. Returns whether the visible list changed.
    pub fn set_query(&mut self, query: &str) -> bool {
        if self.query == query {
            return false;
        }
        self.query.clear();
        self.query.push_str(query);
        let before = std::mem::take(&mut self.visible);
        self.visible = self.matcher.rank(&self.query, &self.rows);
        let changed = before != self.visible;
        if self.is_searching() {
            // The best match is the row at the top: Enter connects what the user sees first.
            self.selected = self.row(0).map(|row| row.entry().id().clone());
        } else {
            self.keep_selection();
        }
        changed
    }

    /// The search text.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Whether a search is active (the text is not blank).
    pub fn is_searching(&self) -> bool {
        !self.query.trim().is_empty()
    }

    /// How many entries the catalog holds, searched or not.
    pub fn total(&self) -> usize {
        self.rows.len()
    }

    /// The header text: how many clusters there are, or how many of them match the search.
    pub fn count_label(&self) -> String {
        if self.is_searching() {
            return format!("{} of {}", self.visible_len(), self.total());
        }
        match self.total() {
            1 => "1 cluster".to_owned(),
            n => format!("{n} clusters"),
        }
    }

    /// How many entries match the query.
    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// The visible row at `ix`, in display order.
    pub fn row(&self, ix: usize) -> Option<&Row> {
        self.visible.get(ix).and_then(|&row| self.rows.get(row))
    }

    /// The visible rows in display order.
    pub fn visible_rows(&self) -> impl Iterator<Item = &Row> {
        self.visible.iter().filter_map(|&ix| self.rows.get(ix))
    }

    /// The names of the visible entries in display order.
    pub fn visible_names(&self) -> Vec<&str> {
        self.visible_rows().map(|row| row.name().as_ref()).collect()
    }

    /// The connection state of `cluster`; `Disconnected` for one with no session.
    pub fn state(&self, cluster: &ClusterId) -> &ClusterSessionState {
        self.states
            .get(cluster)
            .unwrap_or(&ClusterSessionState::Disconnected)
    }

    /// The status badge of the visible row `ix`.
    pub fn badge(&self, ix: usize) -> Option<Badge> {
        let row = self.row(ix)?;
        Some(Badge::of(
            row.entry().context.problem.as_deref(),
            self.state(row.entry().id()),
        ))
    }

    /// Records the connection state of `cluster`. Returns whether it changed.
    pub fn set_state(&mut self, cluster: ClusterId, state: ClusterSessionState) -> bool {
        if self.state(&cluster) == &state {
            return false;
        }
        self.states.insert(cluster, state);
        true
    }

    /// Forgets the session state of `cluster` (its session closed). Returns whether it changed.
    pub fn clear_state(&mut self, cluster: &ClusterId) -> bool {
        self.states.remove(cluster).is_some()
    }

    /// Forgets the state of every cluster `keep` rejects (used to resync after missed session
    /// updates). Returns whether anything was forgotten.
    pub fn retain_states(&mut self, mut keep: impl FnMut(&ClusterId) -> bool) -> bool {
        let before = self.states.len();
        self.states.retain(|cluster, _| keep(cluster));
        self.states.len() != before
    }

    /// Whether the cluster is in the catalog (visible or not).
    pub fn contains(&self, cluster: &ClusterId) -> bool {
        self.rows.iter().any(|row| row.entry().id() == cluster)
    }

    /// Sets a favourite flag now, before the state db has it. Reorders the list. Returns
    /// whether anything changed.
    pub fn set_favourite(&mut self, cluster: &ClusterId, favourite: bool) -> bool {
        let changed = self.update_entry(cluster, |entry| {
            let changed = entry.favourite != favourite;
            entry.favourite = favourite;
            changed
        });
        if changed {
            self.rows.sort_by(|a, b| a.entry().cmp_default(b.entry()));
            self.refilter();
        }
        changed
    }

    /// Sets the last-used time now, before the state db has it. The list is *not* reordered: a
    /// row must not jump under the pointer that just clicked it. The next read of the catalog
    /// sorts it into place. Returns whether anything changed.
    pub fn set_last_used(&mut self, cluster: &ClusterId, at: Timestamp) -> bool {
        self.update_entry(cluster, |entry| {
            let changed = entry.last_used != Some(at);
            entry.last_used = Some(at);
            changed
        })
    }

    /// Applies `change` to the entry of `cluster` and rebuilds its row when `change` returns
    /// `true`. Neither reorders nor refilters.
    fn update_entry(
        &mut self,
        cluster: &ClusterId,
        change: impl FnOnce(&mut CatalogEntry) -> bool,
    ) -> bool {
        let Some(row) = self.rows.iter_mut().find(|r| r.entry().id() == cluster) else {
            return false;
        };
        let mut entry = row.entry().clone();
        if !change(&mut entry) {
            return false;
        }
        *row = Row::new(entry);
        true
    }

    fn refilter(&mut self) {
        self.visible = self.matcher.rank(&self.query, &self.rows);
        self.keep_selection();
    }

    /// Keeps the selected cluster selected if it is still visible; otherwise selects the first
    /// visible entry (or nothing).
    fn keep_selection(&mut self) {
        let still_visible = self
            .selected
            .as_ref()
            .is_some_and(|id| self.visible_rows().any(|row| row.entry().id() == id));
        if !still_visible {
            self.selected = self.row(0).map(|row| row.entry().id().clone());
        }
    }

    /// The selected cluster.
    pub fn selected(&self) -> Option<&ClusterId> {
        self.selected.as_ref()
    }

    /// The display index of the selected cluster.
    pub fn selected_index(&self) -> Option<usize> {
        let id = self.selected.as_ref()?;
        self.visible_rows().position(|row| row.entry().id() == id)
    }

    /// The selected entry.
    pub fn selected_entry(&self) -> Option<&CatalogEntry> {
        let ix = self.selected_index()?;
        self.row(ix).map(Row::entry)
    }

    /// Selects the visible row `ix`. Returns whether the selection changed.
    pub fn select(&mut self, ix: usize) -> bool {
        let Some(id) = self.row(ix).map(|row| row.entry().id().clone()) else {
            return false;
        };
        if self.selected.as_ref() == Some(&id) {
            return false;
        }
        self.selected = Some(id);
        true
    }

    /// Moves the selection by `delta` rows, stopping at the ends. Returns whether it moved.
    pub fn select_by(&mut self, delta: isize) -> bool {
        let len = self.visible.len();
        if len == 0 {
            return false;
        }
        let from = self.selected_index().map_or(0, |ix| ix as isize);
        let to = (from + delta).clamp(0, len as isize - 1) as usize;
        self.select(to)
    }

    /// Selects the first visible row.
    pub fn select_first(&mut self) -> bool {
        self.select(0)
    }

    /// Selects the last visible row.
    pub fn select_last(&mut self) -> bool {
        self.visible
            .len()
            .checked_sub(1)
            .is_some_and(|ix| self.select(ix))
    }
}
