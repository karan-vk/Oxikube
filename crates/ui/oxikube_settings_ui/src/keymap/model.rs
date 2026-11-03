//! [`KeymapListModel`]: the rows of the keymap editor and what is showing of them, as plain data.
//!
//! The view owns one and the table reads it; everything the editor decides (what the search
//! matches, what the context filter and the "unbound" switch hide, which empty state to explain)
//! is here, so it is tested without a window.

use std::collections::BTreeSet;

use oxikube_keymap::{KeymapLayer, KeymapRow, RowStatus};

use super::search::{Haystack, Query};

/// Which context the list is limited to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ContextFilter {
    /// Every context (and the unbound commands).
    #[default]
    All,
    /// Bindings that apply everywhere (no context predicate).
    Everywhere,
    /// Bindings limited to this context expression, spelled as the keymap does.
    Named(String),
}

impl ContextFilter {
    /// What the filter button says.
    pub fn label(&self) -> String {
        match self {
            Self::All => "All contexts".to_owned(),
            Self::Everywhere => "Everywhere".to_owned(),
            Self::Named(name) => name.clone(),
        }
    }
}

/// Why the table has no rows to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyState {
    /// Rows are showing.
    None,
    /// The search or a filter hides every row.
    NoMatches,
    /// The keymap is not loaded, so there is nothing to list.
    NoKeymap,
}

/// Identifies a row across reloads, so the selection survives them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowKey {
    action: String,
    context: Option<String>,
    keystrokes: Vec<String>,
    data: Option<String>,
    source: Option<KeymapLayer>,
    status: RowStatus,
}

impl RowKey {
    /// The key of `row`.
    pub fn of(row: &KeymapRow) -> Self {
        Self {
            action: row.action.clone(),
            context: row.context.clone(),
            keystrokes: row.keystrokes.clone(),
            data: row.data.clone(),
            source: row.source,
            status: row.status,
        }
    }

    /// Whether this is the key of `row`. Allocates nothing: the table asks it of every row it
    /// draws.
    pub fn is(&self, row: &KeymapRow) -> bool {
        self.action == row.action
            && self.context == row.context
            && self.keystrokes == row.keystrokes
            && self.data == row.data
            && self.source == row.source
            && self.status == row.status
    }
}

/// The rows and the filters on them.
#[derive(Debug, Default)]
pub struct KeymapListModel {
    rows: Vec<KeymapRow>,
    haystacks: Vec<Haystack>,
    /// Indexes into `rows` of what shows, in row order.
    visible: Vec<usize>,
    query: Query,
    query_text: String,
    context: ContextFilter,
    show_unbound: bool,
    contexts: Vec<String>,
    has_everywhere: bool,
    has_user_bindings: bool,
}

impl KeymapListModel {
    /// A model over `rows`, unbound commands shown, no filter.
    pub fn new(rows: Vec<KeymapRow>) -> Self {
        let mut model = Self {
            show_unbound: true,
            ..Self::default()
        };
        model.set_rows(rows);
        model
    }

    /// Replace the rows (the keymap was reloaded). The search and the switches stay; a context
    /// filter whose context is gone goes back to "All contexts".
    pub fn set_rows(&mut self, rows: Vec<KeymapRow>) {
        self.haystacks = rows.iter().map(Haystack::of).collect();
        let bound = |row: &&KeymapRow| row.status != RowStatus::Unbound;
        self.contexts = rows
            .iter()
            .filter(bound)
            .filter_map(|row| row.context.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self.has_everywhere = rows.iter().filter(bound).any(|row| row.context.is_none());
        self.has_user_bindings = rows.iter().any(|row| row.source == Some(KeymapLayer::User));
        self.rows = rows;
        let gone = match &self.context {
            ContextFilter::All => false,
            ContextFilter::Everywhere => !self.has_everywhere,
            ContextFilter::Named(name) => !self.contexts.contains(name),
        };
        if gone {
            self.context = ContextFilter::All;
        }
        self.refilter();
    }

    /// Set the search text.
    pub fn set_query(&mut self, text: &str) {
        if self.query_text == text {
            return;
        }
        text.clone_into(&mut self.query_text);
        self.query = Query::parse(text);
        self.refilter();
    }

    /// Limit the list to `context`.
    pub fn set_context(&mut self, context: ContextFilter) {
        if self.context != context {
            self.context = context;
            self.refilter();
        }
    }

    /// Show or hide the commands nothing is bound to.
    pub fn set_show_unbound(&mut self, show: bool) {
        if self.show_unbound != show {
            self.show_unbound = show;
            self.refilter();
        }
    }

    fn refilter(&mut self) {
        let unbound_hidden = !self.show_unbound || self.context != ContextFilter::All;
        self.visible = self
            .rows
            .iter()
            .zip(&self.haystacks)
            .enumerate()
            .filter(|(_, (row, haystack))| {
                if row.status == RowStatus::Unbound && unbound_hidden {
                    return false;
                }
                let in_context = match &self.context {
                    ContextFilter::All => true,
                    ContextFilter::Everywhere => row.context.is_none(),
                    ContextFilter::Named(name) => row.context.as_ref() == Some(name),
                };
                in_context && self.query.matches(haystack)
            })
            .map(|(index, _)| index)
            .collect();
    }

    /// How many rows show.
    pub fn len(&self) -> usize {
        self.visible.len()
    }

    /// Whether no row shows.
    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// The `index`th row that shows.
    pub fn row(&self, index: usize) -> Option<&KeymapRow> {
        self.rows.get(*self.visible.get(index)?)
    }

    /// The position among the rows that show of the row `key` names.
    pub fn position_of(&self, key: &RowKey) -> Option<usize> {
        self.visible.iter().position(|&at| key.is(&self.rows[at]))
    }

    /// How many rows there are, shown or not.
    pub fn total(&self) -> usize {
        self.rows.len()
    }

    /// The search text.
    pub fn query_text(&self) -> &str {
        &self.query_text
    }

    /// The context filter.
    pub fn context(&self) -> &ContextFilter {
        &self.context
    }

    /// Whether unbound commands show.
    pub fn show_unbound(&self) -> bool {
        self.show_unbound
    }

    /// The context expressions that have bindings, sorted.
    pub fn contexts(&self) -> &[String] {
        &self.contexts
    }

    /// Whether some binding applies everywhere (so "Everywhere" is a choice).
    pub fn has_everywhere(&self) -> bool {
        self.has_everywhere
    }

    /// Whether the user's file contributes a binding, a `null` aside. When it does not, the page
    /// points at "Open keymap.json".
    pub fn has_user_bindings(&self) -> bool {
        self.has_user_bindings
    }

    /// Why there is nothing to show, if there is not.
    pub fn empty_state(&self) -> EmptyState {
        if !self.visible.is_empty() {
            EmptyState::None
        } else if self.rows.is_empty() {
            EmptyState::NoKeymap
        } else {
            EmptyState::NoMatches
        }
    }
}
