//! Fuzzy matching of strings for picker delegates, with `nucleo-matcher`.
//!
//! The shape follows Zed's `fuzzy::match_strings` (candidates with an id, matches with a score and
//! the matched positions for highlighting); the code is written from scratch on nucleo. E11-S11
//! moves fuzzy matching into a shared service; delegates call it through
//! [`super::PickerDelegate::update_matches`], so that swap stays local to them.

use std::ops::Range;
use std::sync::Arc;

use gpui::{BackgroundExecutor, HighlightStyle, SharedString, StyledText, Task};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use unicode_segmentation::UnicodeSegmentation;

/// Up to this many candidates are matched on the calling thread: well under a millisecond
/// (`examples/picker_bench.rs`), and no frame shows the previous query's matches. Larger sets are
/// matched on the background executor.
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

/// The candidates that match `query`, best first; equal scores keep the candidates' order. A blank
/// query matches every candidate in order. The query is split on whitespace and every word must
/// match; case is ignored unless the query has capitals. At most `max_results` are returned.
pub fn match_strings(
    candidates: &[StringMatchCandidate],
    query: &str,
    max_results: usize,
) -> Vec<StringMatch> {
    let query = query.trim();
    if query.is_empty() {
        return candidates
            .iter()
            .take(max_results)
            .map(|candidate| StringMatch {
                candidate_id: candidate.id,
                score: 0,
                positions: Vec::new(),
                string: candidate.string.clone(),
            })
            .collect();
    }
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buffer = Vec::new();
    let mut scored: Vec<(u32, usize)> = Vec::new();
    for (ix, candidate) in candidates.iter().enumerate() {
        let haystack = Utf32Str::new(&candidate.string, &mut buffer);
        if let Some(score) = pattern.score(haystack, &mut matcher) {
            scored.push((score, ix));
        }
    }
    // Stable: equal scores keep the candidates' order.
    scored.sort_by_key(|&(score, _)| std::cmp::Reverse(score));
    scored.truncate(max_results);
    let mut indices = Vec::new();
    scored
        .into_iter()
        .map(|(score, ix)| {
            let candidate = &candidates[ix];
            indices.clear();
            let haystack = Utf32Str::new(&candidate.string, &mut buffer);
            let ascii_haystack = matches!(haystack, Utf32Str::Ascii(_));
            pattern.indices(haystack, &mut matcher, &mut indices);
            StringMatch {
                candidate_id: candidate.id,
                score,
                positions: match_indices_to_bytes(&candidate.string, ascii_haystack, &mut indices),
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
    if candidates.len() <= INLINE_MATCH_LIMIT {
        Task::ready(match_strings(&candidates, &query, max_results))
    } else {
        executor.spawn(async move { match_strings(&candidates, &query, max_results) })
    }
}

/// Nucleo's match indices to sorted, deduplicated byte offsets into `text`.
///
/// Nucleo counts in the units of the haystack `Utf32Str::new` built: bytes of `text` when that is
/// [`Utf32Str::Ascii`] (also chosen when every grapheme merely *starts* with an ASCII char, as in
/// `e` + a combining accent), and extended grapheme clusters otherwise (one char per grapheme).
/// Neither is a Rust char index once a grapheme spans several codepoints.
fn match_indices_to_bytes(text: &str, ascii_haystack: bool, indices: &mut Vec<u32>) -> Vec<usize> {
    indices.sort_unstable();
    indices.dedup();
    if ascii_haystack {
        return indices
            .iter()
            .map(|&ix| ix as usize)
            .filter(|&byte| byte < text.len() && text.is_char_boundary(byte))
            .collect();
    }
    let mut wanted = indices.iter().copied().peekable();
    let mut bytes = Vec::with_capacity(indices.len());
    for (grapheme_ix, (byte, _)) in text.grapheme_indices(true).enumerate() {
        match wanted.peek() {
            Some(&next) if next as usize == grapheme_ix => {
                bytes.push(byte);
                wanted.next();
            }
            Some(_) => {}
            None => break,
        }
    }
    bytes
}

/// `text` with the characters at `positions` (byte offsets, as in [`StringMatch::positions`])
/// drawn in `highlight`, each with the rest of its grapheme (a combining accent, say); consecutive
/// characters share one run.
pub fn highlighted_text(
    text: SharedString,
    positions: &[usize],
    highlight: HighlightStyle,
) -> StyledText {
    let ranges = highlight_ranges(&text, positions);
    StyledText::new(text).with_highlights(ranges.into_iter().map(|range| (range, highlight)))
}

/// The byte ranges covering the graphemes that start at `positions`, merged where they touch.
fn highlight_ranges(text: &str, positions: &[usize]) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for &start in positions {
        let Some(grapheme) = text
            .get(start..)
            .and_then(|rest| rest.graphemes(true).next())
        else {
            continue;
        };
        let end = start + grapheme.len();
        match ranges.last_mut() {
            Some(last) if last.end == start => last.end = end,
            _ => ranges.push(start..end),
        }
    }
    ranges
}

/// A latest-wins counter for asynchronous match results: take [`Self::next`] when a query starts,
/// and write its result only while [`Self::is_current`] says it is still the newest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryGeneration(u64);

impl QueryGeneration {
    /// Starts a new query and returns its generation.
    pub fn next(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }

    /// The newest generation handed out.
    pub fn current(&self) -> u64 {
        self.0
    }

    /// Whether `generation` is still the newest query.
    pub fn is_current(&self, generation: u64) -> bool {
        self.0 == generation
    }
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
            highlight_ranges("ünïcode-pod", &found[0].positions),
            [10..13]
        );
    }

    #[test]
    fn positions_count_graphemes_when_an_accent_is_a_combining_mark() {
        // Every grapheme starts with an ASCII char, so nucleo matches the raw bytes.
        let decomposed = "cafe\u{301}-pod";
        let found = match_strings(&candidates(&[decomposed]), "pod", 10);
        assert_eq!(found[0].positions, [7, 8, 9]);
        assert_eq!(highlight_ranges(decomposed, &found[0].positions), [7..10]);

        // A non-ASCII grapheme start: nucleo matches one char per grapheme.
        let mixed = "\u{fc}e\u{301}-pod";
        let found = match_strings(&candidates(&[mixed]), "pod", 10);
        assert_eq!(found[0].positions, [6, 7, 8]);
        assert_eq!(highlight_ranges(mixed, &found[0].positions), [6..9]);
    }

    #[test]
    fn a_highlight_covers_the_whole_grapheme() {
        let found = match_strings(&candidates(&["cafe\u{301}"]), "cafe", 10);
        assert_eq!(found[0].positions, [0, 1, 2, 3]);
        assert_eq!(highlight_ranges("cafe\u{301}", &found[0].positions), [0..6]);
    }

    #[test]
    fn highlight_ranges_merge_neighbours_and_skip_bad_offsets() {
        assert_eq!(highlight_ranges("abcdef", &[0, 1, 3, 99]), [0..2, 3..4]);
        assert_eq!(highlight_ranges("é", &[1]), Vec::<Range<usize>>::new());
    }

    #[test]
    fn generations_are_latest_wins() {
        let mut generation = QueryGeneration::default();
        let first = generation.next();
        let second = generation.next();
        assert!(!generation.is_current(first));
        assert!(generation.is_current(second));
        assert_eq!(generation.current(), second);
    }
}
