//! The page's search: fuzzy over section titles, field titles, paths and descriptions.
//!
//! The ranking is [`FuzzyService`], the one engine the palette, the pickers and the jump bar use
//! (E11-S11); this file only decides what is matched and how the scores of one field combine.
//! The index is built once when the page opens and every keystroke filters it on the UI thread:
//! a few hundred short strings, well under a millisecond (`page::tests::search` has a budget).
//!
//! A title or path matches like a palette entry. A description is long prose in which almost
//! any short query is some subsequence, so a description match counts only when it is tight
//! (most of the query in runs, not one letter per word) and it ranks below a title match.

use std::cmp::Reverse;
use std::ops::Range;

use gpui::SharedString;
use oxikube_app::FuzzyService;
use oxikube_app::search::fuzzy::highlight::matched_ranges;

use super::model::SettingsPageModel;

/// At most this many results are shown.
pub const MAX_HITS: usize = 60;

/// The lowest fuzzy score per query character a description match needs: a tight run of
/// characters scores well above it, letters scattered across unrelated words score below.
const DESCRIPTION_SCORE_PER_CHAR: u32 = 20;

/// Where a hit matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchedIn {
    /// The title (of a field, or of a section).
    Title,
    /// The dotted path of the field (`terminal.font_size`).
    Path,
    /// The description.
    Description,
}

/// One result: a section, or one field of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// The section's index in the model.
    pub section: usize,
    /// The field's index in its section; `None` for the section itself.
    pub field: Option<usize>,
    /// Higher is better.
    pub score: u32,
    /// Where it matched best.
    pub matched_in: MatchedIn,
    /// Byte ranges of the title to highlight (empty unless it matched in the title).
    pub title_ranges: Vec<Range<usize>>,
}

/// One searchable row: a section or a field.
#[derive(Clone, Debug)]
struct Entry {
    section: usize,
    field: Option<usize>,
    title: SharedString,
    path: SharedString,
    description: SharedString,
}

/// What the search runs over, built once per page open. See the [page module](crate::page).
#[derive(Clone, Debug, Default)]
pub struct SearchIndex {
    entries: Vec<Entry>,
}

impl SearchIndex {
    /// The index of every section and field of `model`.
    pub fn new(model: &SettingsPageModel) -> Self {
        let mut entries = Vec::with_capacity(model.field_count() + model.sections().len());
        for (section_ix, section) in model.sections().iter().enumerate() {
            entries.push(Entry {
                section: section_ix,
                field: None,
                title: section.title.clone(),
                path: section.key.clone(),
                description: section.description.clone().unwrap_or_default(),
            });
            for (field_ix, field) in section.fields.iter().enumerate() {
                entries.push(Entry {
                    section: section_ix,
                    field: Some(field_ix),
                    title: field.title.clone(),
                    path: field.path.clone(),
                    description: field.description.clone(),
                });
            }
        }
        Self { entries }
    }

    /// The sections and fields that match `query`, best first (ties keep page order). A blank
    /// query matches nothing: the page shows its sections instead.
    pub fn search(&self, query: &str) -> Vec<SearchHit> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let fuzzy = FuzzyService::shared();
        let all = self.entries.len();
        let mut best: Vec<Option<SearchHit>> = vec![None; all];
        let mut offer = |index: usize, score: u32, matched_in: MatchedIn, ranges| {
            let entry = &self.entries[index];
            let slot = &mut best[index];
            if slot.as_ref().is_none_or(|hit| score > hit.score) {
                *slot = Some(SearchHit {
                    section: entry.section,
                    field: entry.field,
                    score,
                    matched_in,
                    title_ranges: ranges,
                });
            }
        };
        for found in fuzzy.rank_with(query, &self.entries, all, |e| &e.title, |_| None) {
            let ranges = matched_ranges(&self.entries[found.index].title, &found.positions);
            offer(found.index, found.score, MatchedIn::Title, ranges);
        }
        for found in fuzzy.rank_with(query, &self.entries, all, |e| &e.path, |_| None) {
            // A path names the section too (`term font`), so it ranks just below the title.
            offer(
                found.index,
                found.score.saturating_sub(1),
                MatchedIn::Path,
                Vec::new(),
            );
        }
        let needed = DESCRIPTION_SCORE_PER_CHAR * query_chars(query);
        for found in fuzzy.rank_with(query, &self.entries, all, |e| &e.description, |_| None) {
            if found.score >= needed {
                offer(
                    found.index,
                    found.score / 2,
                    MatchedIn::Description,
                    Vec::new(),
                );
            }
        }
        let mut hits: Vec<SearchHit> = best.into_iter().flatten().collect();
        // Stable: equal scores keep the page's order.
        hits.sort_by_key(|hit| Reverse(hit.score));
        hits.truncate(MAX_HITS);
        hits
    }

    /// How many sections and fields the index holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index holds nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The characters of `query` that must match (whitespace separates words and matches nothing).
fn query_chars(query: &str) -> u32 {
    let count = query.chars().filter(|ch| !ch.is_whitespace()).count();
    u32::try_from(count).unwrap_or(u32::MAX)
}
