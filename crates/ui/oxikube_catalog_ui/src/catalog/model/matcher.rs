//! Fuzzy filtering of the catalog with `nucleo-matcher`.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::Row;

/// How much a match in the name counts on top of a match anywhere in the row, so `prod` ranks
/// the context called `prod-eu` above one that merely lives in a file called `prod.yaml`.
const NAME_WEIGHT: u32 = 2;

/// The matcher and its scratch buffers, reused across keystrokes (no allocation per entry).
pub(super) struct FuzzyMatcher {
    matcher: Matcher,
    buffer: Vec<char>,
    scored: Vec<(u32, usize)>,
}

impl FuzzyMatcher {
    pub(super) fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            buffer: Vec::new(),
            scored: Vec::new(),
        }
    }

    /// The indexes of the `rows` that match `query`, best first; rows with equal scores keep
    /// their order in `rows`. A blank query matches everything in its order.
    ///
    /// The query is split on whitespace and every word must match somewhere in the row (name,
    /// cluster, user or source), in any order. Case is ignored unless the query has capitals.
    pub(super) fn rank(&mut self, query: &str, rows: &[Row]) -> Vec<usize> {
        let query = query.trim();
        if query.is_empty() {
            return (0..rows.len()).collect();
        }
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        self.scored.clear();
        for (ix, row) in rows.iter().enumerate() {
            let haystack = Utf32Str::new(row.haystack(), &mut self.buffer);
            let Some(score) = pattern.score(haystack, &mut self.matcher) else {
                continue;
            };
            let name = Utf32Str::new(row.name(), &mut self.buffer);
            let in_name = pattern.score(name, &mut self.matcher).unwrap_or(0);
            self.scored.push((score + NAME_WEIGHT * in_name, ix));
        }
        // Stable: equal scores stay in the default order (favourites, last used, name).
        self.scored
            .sort_by_key(|&(score, _)| std::cmp::Reverse(score));
        self.scored.iter().map(|&(_, ix)| ix).collect()
    }
}
