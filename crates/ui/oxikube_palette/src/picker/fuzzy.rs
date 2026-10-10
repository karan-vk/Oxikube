//! Fuzzy matching of strings for picker delegates: the picker-shaped face of
//! [`oxikube_app::FuzzyService`], the one ranking engine the palette, the pickers and the jump bar
//! share (E11-S11, built on `nucleo-matcher`).
//!
//! The shape follows Zed's `fuzzy::match_strings` (candidates with an id, matches with a score and
//! the matched positions for highlighting); the ranking itself lives in `oxikube_app` (plain Rust,
//! tested with fakes) and this file only adapts the types and decides, per list, whether the match
//! runs on the calling thread or on the background executor.

use std::sync::Arc;

use gpui::{BackgroundExecutor, HighlightStyle, SharedString, StyledText, Task};
use oxikube_app::FuzzyService;
use oxikube_app::search::fuzzy::highlight;

pub use oxikube_app::QueryGeneration;

/// Up to this many candidates are matched on the calling thread: well under a millisecond
/// (`examples/picker_bench.rs`, `cargo bench -p oxikube_app --bench fuzzy_rank`), and no frame
/// shows the previous query's matches. Larger sets are matched on the background executor.
pub const INLINE_MATCH_LIMIT: usize = 512;

/// A string to match, with the caller's id for it (its index in the caller's list, usually).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringMatchCandidate {
    /// The caller's id, returned in [`StringMatch::candidate_id`].
    pub id: usize,
    /// The text matched against.
    pub string: SharedString,
}

impl StringMatchCandidate {
    /// A candidate `string` known to the caller as `id`.
    pub fn new(id: usize, string: impl Into<SharedString>) -> Self {
        Self {
            id,
            string: string.into(),
        }
    }
}

/// A candidate that matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringMatch {
    /// The candidate's id.
    pub candidate_id: usize,
    /// Higher is better; 0 for the blank query.
    pub score: u32,
    /// Byte offsets of the matched characters in [`Self::string`], ascending.
    pub positions: Vec<usize>,
    /// The candidate's text.
    pub string: SharedString,
}

/// The candidates that match `query`, best first; equal scores order alphabetically (see
/// [`FuzzyService`] for the rules). A blank query matches every candidate in order. The query is
/// split on whitespace and every word must match; case is ignored unless the query has capitals.
/// At most `max_results` are returned.
pub fn match_strings(
    candidates: &[StringMatchCandidate],
    query: &str,
    max_results: usize,
) -> Vec<StringMatch> {
    match_strings_by(candidates, query, max_results, |_| None)
}

/// [`match_strings`] with a boost for the candidates used lately: `recency` maps a candidate's id
/// to its place among the recents (`Some(0)` is the latest) and decides between near-equal
/// matches; a blank query lists the recent candidates first.
pub fn match_strings_by(
    candidates: &[StringMatchCandidate],
    query: &str,
    max_results: usize,
    recency: impl Fn(usize) -> Option<usize>,
) -> Vec<StringMatch> {
    FuzzyService::shared()
        .rank_with(
            query,
            candidates,
            max_results,
            |candidate| candidate.string.as_ref(),
            |candidate| recency(candidate.id),
        )
        .into_iter()
        .map(|found| {
            let candidate = &candidates[found.index];
            StringMatch {
                candidate_id: candidate.id,
                score: found.score,
                positions: found.positions,
                string: candidate.string.clone(),
            }
        })
        .collect()
}

/// [`match_strings`] as a task: inline (a ready task) for up to [`INLINE_MATCH_LIMIT`] candidates,
/// on `executor` above that, so a keystroke never blocks the UI thread on a large list.
pub fn match_strings_async(
    candidates: Arc<[StringMatchCandidate]>,
    query: String,
    max_results: usize,
    executor: &BackgroundExecutor,
) -> Task<Vec<StringMatch>> {
    match_strings_async_by(candidates, query, max_results, |_| None, executor)
}

/// [`match_strings_by`] as a task, placed like [`match_strings_async`].
pub fn match_strings_async_by(
    candidates: Arc<[StringMatchCandidate]>,
    query: String,
    max_results: usize,
    recency: impl Fn(usize) -> Option<usize> + Send + 'static,
    executor: &BackgroundExecutor,
) -> Task<Vec<StringMatch>> {
    if candidates.len() <= INLINE_MATCH_LIMIT {
        Task::ready(match_strings_by(&candidates, &query, max_results, recency))
    } else {
        executor.spawn(async move { match_strings_by(&candidates, &query, max_results, recency) })
    }
}

/// `text` with the characters at `positions` (byte offsets, as in [`StringMatch::positions`])
/// drawn in `highlight`, each with the rest of its grapheme (a combining accent, say); consecutive
/// characters share one run.
pub fn highlighted_text(
    text: SharedString,
    positions: &[usize],
    style: HighlightStyle,
) -> StyledText {
    let ranges = highlight::matched_ranges(&text, positions);
    StyledText::new(text).with_highlights(ranges.into_iter().map(|range| (range, style)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(names: &[&str]) -> Vec<StringMatchCandidate> {
        names
            .iter()
            .enumerate()
            .map(|(ix, name)| StringMatchCandidate::new(ix, name.to_string()))
            .collect()
    }

    #[test]
    fn a_blank_query_keeps_every_candidate_in_order() {
        let list = candidates(&["b", "a", "c"]);
        let ids: Vec<_> = match_strings(&list, "  ", 10)
            .iter()
            .map(|m| m.candidate_id)
            .collect();
        assert_eq!(ids, [0, 1, 2]);
        assert_eq!(match_strings(&list, "", 2).len(), 2, "capped");
    }

    #[test]
    fn matches_rank_best_first_and_report_byte_positions() {
        let list = candidates(&["log-shipper", "proxy", "app"]);
        let found = match_strings(&list, "pp", 10);
        let ids: Vec<_> = found.iter().map(|m| m.candidate_id).collect();
        assert!(ids.contains(&0) && ids.contains(&2), "{ids:?}");
        assert!(!ids.contains(&1), "proxy has one p");
        let app = found.iter().find(|m| m.candidate_id == 2).unwrap();
        assert_eq!(app.positions, [1, 2]);
    }

    #[test]
    fn every_word_must_match_and_case_is_smart() {
        let list = candidates(&["kube-system coredns", "default nginx", "Kube"]);
        let ids: Vec<_> = match_strings(&list, "kube dns", 10)
            .iter()
            .map(|m| m.candidate_id)
            .collect();
        assert_eq!(ids, [0]);
        let ids: Vec<_> = match_strings(&list, "Kube", 10)
            .iter()
            .map(|m| m.candidate_id)
            .collect();
        assert_eq!(ids, [2], "a capital makes the match case-sensitive");
    }

    #[test]
    fn positions_are_byte_offsets_for_multibyte_text() {
        let list = candidates(&["ünïcode-pod"]);
        let found = match_strings(&list, "pod", 10);
        assert_eq!(found[0].positions, [10, 11, 12]);
        assert_eq!(
            highlight::matched_ranges("ünïcode-pod", &found[0].positions),
            [10..13]
        );
    }

    #[test]
    fn recents_lead_a_blank_query_and_decide_near_ties() {
        let list = candidates(&["Pod Delete", "Pod Describe", "Node Drain"]);
        let recent = |id: usize| (id == 2).then_some(0);
        let blank: Vec<_> = match_strings_by(&list, "", 10, recent)
            .iter()
            .map(|m| m.candidate_id)
            .collect();
        assert_eq!(blank, [2, 0, 1]);
        let typed = match_strings_by(&list, "pod de", 10, |id| (id == 1).then_some(0));
        assert_eq!(
            typed[0].candidate_id, 1,
            "the recent one of two equal matches"
        );
    }
}
