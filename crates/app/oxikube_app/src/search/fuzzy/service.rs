//! [`FuzzyService`]: ranking candidates for a query with `nucleo-matcher`.

use std::cmp::Ordering;
use std::sync::OnceLock;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use parking_lot::Mutex;
use unicode_segmentation::UnicodeSegmentation;

use super::score;

/// A candidate that matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// The candidate's index in the slice that was ranked.
    pub index: usize,
    /// Higher is better. `0` for a blank query, apart from the recents boost.
    pub score: u32,
    /// Byte offsets of the matched characters in the candidate's text, ascending, one per
    /// character (the start of its grapheme when the text has combining marks; what [`highlight`](super::highlight) takes). Empty for a blank query.
    pub positions: Vec<usize>,
}

/// One worker's matcher and its scratch buffers.
struct Scratch {
    matcher: Matcher,
    /// The candidate being matched, as the chars nucleo wants for non-ASCII text.
    chars: Vec<char>,
    /// Nucleo's indices of a match, before they become byte offsets.
    indices: Vec<u32>,
}

impl Scratch {
    fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            chars: Vec::new(),
            indices: Vec::new(),
        }
    }
}

/// Lends a [`Scratch`] from the pool and puts it back, even if the borrower panics.
struct Lease<'a> {
    pool: &'a Mutex<Vec<Scratch>>,
    scratch: Option<Scratch>,
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Some(scratch) = self.scratch.take() {
            self.pool.lock().push(scratch);
        }
    }
}

/// The fuzzy ranking service. See the [module docs](super) for the rules.
///
/// Cheap to create and `Sync`: share one (`FuzzyService::shared`) between the palette, the pickers
/// and the jump bar's completions.
#[derive(Default)]
pub struct FuzzyService {
    /// Idle matchers. A call takes one (or makes one when every matcher is busy) and returns it;
    /// the lock is held only for that swap, never while matching.
    pool: Mutex<Vec<Scratch>>,
}

impl FuzzyService {
    /// A service with an empty pool; matchers are made on first use.
    pub fn new() -> Self {
        Self::default()
    }

    /// The process-wide service the app's surfaces share.
    pub fn shared() -> &'static FuzzyService {
        static SHARED: OnceLock<FuzzyService> = OnceLock::new();
        SHARED.get_or_init(FuzzyService::new)
    }

    fn lease(&self) -> Lease<'_> {
        let scratch = self.pool.lock().pop().unwrap_or_else(Scratch::new);
        Lease {
            pool: &self.pool,
            scratch: Some(scratch),
        }
    }

    /// The best `limit` of `candidates` for `query`, best first. See the [module docs](super).
    pub fn rank<S: AsRef<str>>(&self, query: &str, candidates: &[S], limit: usize) -> Vec<Match> {
        self.rank_with(query, candidates, limit, |s| s.as_ref(), |_| None)
    }

    /// [`rank`](Self::rank) over any items: `text` is what the query is matched against and
    /// `recency` is the item's place among the recents (`Some(0)` is the latest), if it is one.
    pub fn rank_with<T>(
        &self,
        query: &str,
        items: &[T],
        limit: usize,
        text: impl Fn(&T) -> &str,
        recency: impl Fn(&T) -> Option<usize>,
    ) -> Vec<Match> {
        if limit == 0 || items.is_empty() {
            return Vec::new();
        }
        let query = query.trim();
        if query.is_empty() {
            return rank_blank(items, limit, &recency);
        }
        let case_sensitive = score::is_case_sensitive(query);
        let pattern = Pattern::new(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut lease = self.lease();
        let scratch = lease.scratch.as_mut().expect("leased scratch");

        let mut scored: Vec<(u32, usize)> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let haystack = text(item);
            let haystack_chars = Utf32Str::new(haystack, &mut scratch.chars);
            let Some(fuzzy) = pattern.score(haystack_chars, &mut scratch.matcher) else {
                continue;
            };
            let shape = score::shape(haystack, query, case_sensitive);
            scored.push((score::combine(fuzzy, shape, recency(item)), index));
        }
        let by_rank = |a: &(u32, usize), b: &(u32, usize)| {
            order(a, b, |ix| (text(&items[ix]), recency(&items[ix])))
        };
        keep_best(&mut scored, limit, by_rank);

        scored
            .into_iter()
            .map(|(score, index)| {
                scratch.indices.clear();
                let haystack = text(&items[index]);
                let haystack_chars = Utf32Str::new(haystack, &mut scratch.chars);
                let ascii_haystack = matches!(haystack_chars, Utf32Str::Ascii(_));
                pattern.indices(haystack_chars, &mut scratch.matcher, &mut scratch.indices);
                Match {
                    index,
                    score,
                    positions: match_indices_to_bytes(
                        haystack,
                        ascii_haystack,
                        &mut scratch.indices,
                    ),
                }
            })
            .collect()
    }
}

impl std::fmt::Debug for FuzzyService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FuzzyService")
            .field("idle_matchers", &self.pool.lock().len())
            .finish()
    }
}

/// The blank query: every candidate in the caller's order, the recent ones first.
fn rank_blank<T>(items: &[T], limit: usize, recency: &impl Fn(&T) -> Option<usize>) -> Vec<Match> {
    let mut found: Vec<(u32, usize)> = items
        .iter()
        .enumerate()
        .map(|(index, item)| (score::recent_bonus(recency(item)), index))
        .collect();
    // Not `score > 0`: the bonus is spent by rank 12, but the recents run to rank 50.
    if items.iter().any(|item| recency(item).is_some()) {
        // Stable: the candidates that are not recent keep the caller's order.
        keep_best(&mut found, limit, |a, b| {
            let (a_recency, b_recency) = (recency(&items[a.1]), recency(&items[b.1]));
            by_recency(a_recency, b_recency).then(a.1.cmp(&b.1))
        });
    } else {
        found.truncate(limit);
    }
    found
        .into_iter()
        .map(|(score, index)| Match {
            index,
            score,
            positions: Vec::new(),
        })
        .collect()
}

/// Higher score first, then the more recent, then alphabetical, then the caller's order.
fn order<'a>(
    a: &(u32, usize),
    b: &(u32, usize),
    of: impl Fn(usize) -> (&'a str, Option<usize>),
) -> Ordering {
    b.0.cmp(&a.0)
        .then_with(|| {
            let ((a_text, a_recency), (b_text, b_recency)) = (of(a.1), of(b.1));
            by_recency(a_recency, b_recency).then_with(|| score::alphabetical(a_text, b_text))
        })
        .then(a.1.cmp(&b.1))
}

/// The more recent first; a candidate that is not recent comes after every recent one.
fn by_recency(a: Option<usize>, b: Option<usize>) -> Ordering {
    a.unwrap_or(usize::MAX).cmp(&b.unwrap_or(usize::MAX))
}

/// Keeps the best `limit` of `scored` under `cmp`, sorted. Selecting before sorting keeps a
/// small `limit` over a long list from sorting all of it.
fn keep_best<T>(scored: &mut Vec<T>, limit: usize, cmp: impl Fn(&T, &T) -> Ordering) {
    if limit < scored.len() {
        scored.select_nth_unstable_by(limit - 1, &cmp);
        scored.truncate(limit);
    }
    scored.sort_by(cmp);
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
