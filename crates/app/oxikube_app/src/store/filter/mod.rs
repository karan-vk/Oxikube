//! The `/` filter of a resource table (E07-S04): the grammar, the compiled name predicates, and
//! how a parsed filter splits into what the store does client-side and what the server does.
//!
//! | Input | Meaning | Where it runs |
//! |---|---|---|
//! | `foo`, `/foo` | names matching the regex (or substring) `foo`, case-insensitive | in the store, on the cached objects |
//! | `!foo` | names that do not match | in the store |
//! | `-l app=web` | label selector, kubectl syntax | **on the server**: the feed is re-keyed with the selector |
//! | `-f wb` | fuzzy: `wb` as a subsequence of the name, best match first | in the store, ranked by [`Fuzzy::score`] |
//! | `foo -l app=web` | a name filter and a selector together | both, as above |
//!
//! [`parse`] is pure and compiles the pattern once per edit. [`FilterExpr::parts`] turns the
//! expression into [`FilterParts`]: a [`StoreFilter`] (text, inverse and fuzzy are client-side,
//! incremental) and the label selector the subscription hands the server with
//! [`Subscription::set_selector`](super::Subscription::set_selector). The filter never touches
//! the subscription's scope: it composes with the session's namespace selection and cannot
//! widen what is watched.
//!
//! A fuzzy filter ranks: [`FilterParts::sort`] says which [`SortKey`] shows the best match first,
//! and ties break on the object key, so the order is deterministic.

mod fuzzy;
mod name;
mod parse;

#[cfg(test)]
mod tests;

pub use fuzzy::Fuzzy;
pub use name::{NameFilter, NameMatcher, TextPattern};
pub use parse::{FilterError, FilterExpr, MAX_FILTER_LEN, parse};

use super::query::StoreFilter;
use super::selector::LabelSelector;
use super::sort::{SortField, SortKey};

/// A parsed filter split by where it runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterParts {
    /// Applied by the store to the cached objects.
    pub filter: StoreFilter,
    /// Applied by the server: the feed is re-keyed with it. `None` for every object.
    pub selector: Option<LabelSelector>,
}

impl FilterParts {
    /// Whether the filter lets everything through.
    pub fn is_empty(&self) -> bool {
        self.filter.is_empty() && self.selector.is_none()
    }

    /// Whether the matches are ranked (a fuzzy filter), so the list is best-match-first unless
    /// the user sorts by a column.
    pub fn ranks(&self) -> bool {
        self.filter.pattern.as_ref().is_some_and(NameFilter::ranks)
    }

    /// The sort to subscribe with: `chosen` (the user's column sort, if any), else best match
    /// first when this filter ranks, else the store's default order.
    pub fn sort(&self, chosen: Option<SortKey>) -> SortKey {
        match chosen {
            Some(key) => key,
            None if self.ranks() => SortKey::by(SortField::Relevance),
            None => SortKey::default(),
        }
    }
}

impl FilterExpr {
    /// The expression as the store's filter and the server's selector.
    pub fn parts(&self) -> FilterParts {
        let mut parts = FilterParts::default();
        self.fill(&mut parts, false);
        parts
    }

    fn fill(&self, parts: &mut FilterParts, inverse: bool) {
        let matcher = match self {
            FilterExpr::Empty => return,
            FilterExpr::Inverse(inner) => return inner.fill(parts, !inverse),
            // A selector cannot be inverted (parse refuses): it is applied as written.
            FilterExpr::LabelSelector(selector) => {
                parts.selector = Some(selector.clone()).filter(|s| !s.is_empty());
                return;
            }
            FilterExpr::Narrowed { name, selector } => {
                parts.selector = Some(selector.clone()).filter(|s| !s.is_empty());
                return name.fill(parts, inverse);
            }
            FilterExpr::Text(text) => NameMatcher::Text(text.clone()),
            FilterExpr::Fuzzy(fuzzy) => NameMatcher::Fuzzy(fuzzy.clone()),
        };
        let filter = NameFilter::new(matcher);
        let filter = if inverse { filter.inverted() } else { filter };
        parts.filter.pattern = Some(filter).filter(|f| !f.is_empty());
    }
}
