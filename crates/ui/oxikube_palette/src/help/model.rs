//! [`HelpModel`]: the bindings the overlay lists, as plain data.
//!
//! Built once when the overlay opens, from the keymap and the focused view's key-context stack
//! (never per frame), by [`HelpModel::build`], a pure function of the resolved bindings. Grouping
//! and filtering are pure too ([`HelpModel::rows`]), so the overlay's behaviour is tested without
//! a window.

use std::sync::Arc;

use gpui::{SharedString, Window};
use oxikube_keymap::{
    ActiveBinding, SuppressedBinding, active_bindings, all_bindings, suppressed_bindings,
};

use super::entry::{HelpCategory, HelpEntry};
use crate::picker::fuzzy::{StringMatch, StringMatchCandidate, match_strings};

/// What the list is limited to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelpScope {
    /// The keys in force where the focus is; `innermost` is the focused view's own context name
    /// (`ResourceTable`), for the caption.
    Focused {
        /// The deepest key context of the focus path.
        innermost: SharedString,
    },
    /// Nothing with a key context has the focus (before a cluster tab exists): every binding.
    Everything,
}

/// One row of the list: a group header or a binding.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// The heading of a group, with how many bindings it lists under the current query.
    Header {
        /// The group.
        category: HelpCategory,
        /// How many entries follow it.
        count: usize,
    },
    /// A binding: its index in [`HelpModel::entries`], and the byte offsets of the query's match
    /// in the entry's title (for highlighting).
    Entry {
        /// Index into [`HelpModel::entries`].
        entry: usize,
        /// Matched bytes of the title, ascending.
        positions: Vec<usize>,
    },
}

/// The listed bindings, sorted by category, then title, then keys.
#[derive(Clone, Debug)]
pub struct HelpModel {
    scope: HelpScope,
    entries: Vec<HelpEntry>,
    /// One candidate per entry and searchable field (title, category, keys, action name, chips):
    /// a one-word query must match inside a single field, so `log` does not match a title with an
    /// `l`, a category with an `o` and a key with a `g`.
    fields: Arc<[StringMatchCandidate]>,
    /// One candidate per entry holding every field, for queries of several words (`view yaml`,
    /// `pod logs`), whose words may match different fields.
    combined: Arc<[StringMatchCandidate]>,
}

/// How many candidates [`HelpModel::candidates_for`] makes per entry in the per-field list.
const FIELDS: usize = 5;

impl HelpModel {
    /// The model for the focused view of `window`: the keys in force along its key-context stack
    /// (and the defaults the user unbound there), or every binding when the stack is empty.
    pub fn capture(window: &Window, cx: &gpui::App) -> Self {
        let stack = window.context_stack();
        if stack.is_empty() {
            return Self::build(HelpScope::Everything, all_bindings(cx), Vec::new());
        }
        let innermost = stack
            .iter()
            .rev()
            .find_map(|context| context.primary())
            .map(|entry| entry.key.clone())
            .unwrap_or_default();
        Self::build(
            HelpScope::Focused { innermost },
            active_bindings(cx, &stack),
            suppressed_bindings(cx, &stack),
        )
    }

    /// The model of `active` bindings plus the `suppressed` ones, limited to `scope`.
    pub fn build(
        scope: HelpScope,
        active: Vec<ActiveBinding>,
        suppressed: Vec<SuppressedBinding>,
    ) -> Self {
        let mut entries: Vec<HelpEntry> = active
            .iter()
            .map(HelpEntry::active)
            .chain(suppressed.iter().map(HelpEntry::unbound))
            .collect();
        entries.sort_by(|a, b| {
            a.category
                .cmp(&b.category)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                .then_with(|| a.keystrokes.cmp(&b.keystrokes))
        });
        let combined = entries
            .iter()
            .enumerate()
            .map(|(ix, entry)| StringMatchCandidate::new(ix, entry.haystack()))
            .collect();
        let fields = entries
            .iter()
            .enumerate()
            .flat_map(|(ix, entry)| {
                entry
                    .fields()
                    .into_iter()
                    .enumerate()
                    .map(move |(field, text)| StringMatchCandidate::new(ix * FIELDS + field, text))
            })
            .collect();
        Self {
            scope,
            entries,
            fields,
            combined,
        }
    }

    /// What the list is limited to.
    pub fn scope(&self) -> &HelpScope {
        &self.scope
    }

    /// The bindings, in display order.
    pub fn entries(&self) -> &[HelpEntry] {
        &self.entries
    }

    /// The strings `query` is matched against: the per-field strings for a query of one word, the
    /// combined strings of each entry for several words (or none: every entry, in display
    /// order). Pass the matches of exactly these to [`Self::rows`].
    pub fn candidates_for(&self, query: &str) -> Arc<[StringMatchCandidate]> {
        if Self::per_field(query) {
            self.fields.clone()
        } else {
            self.combined.clone()
        }
    }

    /// Whether `query` is matched field by field (a single word).
    fn per_field(query: &str) -> bool {
        query.split_whitespace().count() == 1
    }

    /// The rows for `query`, matched on the calling thread (what the overlay does for a short
    /// list; a long one matches [`Self::candidates_for`] on an executor and calls [`Self::rows`]).
    pub fn search(&self, query: &str) -> Vec<Row> {
        let candidates = self.candidates_for(query);
        self.rows(query, &match_strings(&candidates, query, usize::MAX))
    }

    /// The rows for the `matches` of `query` against [`Self::candidates_for`]: an entry is as good as its best
    /// field, its title highlighted where the title matched. Grouped under a header per category,
    /// headers in category order, the entries of a group best match first (display order for a
    /// blank query).
    pub fn rows(&self, query: &str, matches: &[StringMatch]) -> Vec<Row> {
        let folded = self.fold(matches, Self::per_field(query));
        let mut groups: Vec<(HelpCategory, Vec<&Folded>)> = Vec::new();
        for found in &folded {
            let Some(entry) = self.entries.get(found.entry) else {
                continue;
            };
            match groups.binary_search_by_key(&entry.category, |(category, _)| *category) {
                Ok(ix) => groups[ix].1.push(found),
                Err(ix) => groups.insert(ix, (entry.category, vec![found])),
            }
        }
        let mut rows = Vec::with_capacity(folded.len() + groups.len());
        for (category, found) in groups {
            rows.push(Row::Header {
                category,
                count: found.len(),
            });
            for found in found {
                rows.push(Row::Entry {
                    entry: found.entry,
                    positions: found.title_positions.clone(),
                });
            }
        }
        rows
    }

    /// One [`Folded`] per entry that matched, best first. `per_field`: the ids are
    /// `entry * FIELDS + field` (field 0 is the title), else they are the entries' indexes.
    fn fold(&self, matches: &[StringMatch], per_field: bool) -> Vec<Folded> {
        let mut folded: Vec<Folded> = Vec::new();
        let mut at: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        for found in matches {
            let (entry, field) = if per_field {
                (found.candidate_id / FIELDS, found.candidate_id % FIELDS)
            } else {
                (found.candidate_id, 0)
            };
            let slot = *at.entry(entry).or_insert_with(|| {
                folded.push(Folded {
                    entry,
                    title_positions: Vec::new(),
                });
                folded.len() - 1
            });
            if field == 0 {
                let title_len = self.entries.get(entry).map_or(0, |e| e.title.len());
                folded[slot].title_positions = found
                    .positions
                    .iter()
                    .copied()
                    .take_while(|position| *position < title_len)
                    .collect();
            }
        }
        folded
    }
}

/// An entry that matched, with where its title matched.
struct Folded {
    entry: usize,
    title_positions: Vec<usize>,
}
