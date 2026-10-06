//! [`Fuzzy`]: the `/-f` matcher. A name matches when the query's characters appear in it in
//! order (a subsequence, case-insensitive); the score ranks the matches.
//!
//! The score follows the usual fuzzy-finder shape: every matched character earns a base score,
//! consecutive characters and characters at the start of a word (after `-`, `_`, `.`, `/`, `:`
//! or at the start of the name) earn a bonus, and characters skipped between two matches cost a
//! small penalty. The best alignment wins (dynamic programming over the name, `O(name * query)`),
//! so the score is a pure function of the two strings: ranking is deterministic and the store
//! breaks ties on the object key.

/// Score of one matched character.
const MATCH: i32 = 16;
/// Bonus when a match directly follows the previous one.
const CONSECUTIVE: i32 = 18;
/// Bonus when a match starts a word of the name.
const BOUNDARY: i32 = 10;
/// Extra bonus when the first query character matches the first character of the name.
const PREFIX: i32 = 8;
/// Penalty per name character skipped between two matches (or before the first one).
const GAP: i32 = 3;
/// "No alignment" in the score rows (far below any real score).
const NONE: i32 = i32::MIN / 2;

/// A compiled fuzzy query. Build one per edit of the filter, not per row.
#[derive(Debug, Clone)]
pub struct Fuzzy {
    /// The query folded to lower case, one entry per character.
    needle: Vec<char>,
}

impl Fuzzy {
    /// Compiles `query` (surrounding whitespace is ignored; inner whitespace never matches a
    /// name, so it is dropped).
    pub fn new(query: &str) -> Self {
        let needle = query
            .chars()
            .filter(|c| !c.is_whitespace())
            .map(fold)
            .collect();
        Self { needle }
    }

    /// Whether the query is empty (matches every name).
    pub fn is_empty(&self) -> bool {
        self.needle.is_empty()
    }

    /// Whether `name` contains the query as a subsequence.
    pub fn matches(&self, name: &str) -> bool {
        is_subsequence(&self.needle, name.chars().map(fold))
    }

    /// The score of the best alignment of the query in `name`; `None` when it does not match.
    /// Higher is better. An empty query scores `0` for every name.
    pub fn score(&self, name: &str) -> Option<i32> {
        if self.needle.is_empty() {
            return Some(0);
        }
        let hay: Vec<char> = name.chars().map(fold).collect();
        let starts = word_starts(name);
        let (n, m) = (hay.len(), self.needle.len());
        if m > n {
            return None;
        }
        // `row[j]`: the best score of matching needle[..=i] with needle[i] at hay[j].
        let mut row = vec![NONE; n];
        for (j, c) in hay.iter().enumerate() {
            if *c == self.needle[0] {
                let prefix = if j == 0 { PREFIX } else { 0 };
                row[j] = MATCH + bonus(&starts, j) + prefix - gap(j);
            }
        }
        let mut next = vec![NONE; n];
        for i in 1..m {
            next.fill(NONE);
            // The best `row[k] + GAP * k` over the k that leave a gap before j (k <= j - 2).
            let mut gapped = NONE;
            for j in i..n {
                if j >= 2 && row[j - 2] > NONE {
                    gapped = gapped.max(row[j - 2] + gap(j - 2));
                }
                if hay[j] != self.needle[i] {
                    continue;
                }
                let mut best = NONE;
                if row[j - 1] > NONE {
                    best = row[j - 1] + CONSECUTIVE;
                }
                if gapped > NONE {
                    best = best.max(gapped - gap(j - 1));
                }
                if best > NONE {
                    next[j] = best + MATCH + bonus(&starts, j);
                }
            }
            std::mem::swap(&mut row, &mut next);
        }
        row.into_iter().filter(|s| *s > NONE).max()
    }

    /// Whether every name this query matches is also matched by `older`: `older`'s characters
    /// are a subsequence of this query's (typing more characters only narrows the matches).
    pub fn narrows(&self, older: &Fuzzy) -> bool {
        is_subsequence(&older.needle, self.needle.iter().copied())
    }
}

impl PartialEq for Fuzzy {
    fn eq(&self, other: &Self) -> bool {
        self.needle == other.needle
    }
}

impl Eq for Fuzzy {}

/// Folds a character to lower case (the first character of its lower-case form, so the fold is
/// one-to-one and the positions of a name stay aligned).
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Whether `needle` occurs in `hay` as a subsequence.
fn is_subsequence(needle: &[char], hay: impl Iterator<Item = char>) -> bool {
    let mut want = needle.iter().peekable();
    for c in hay {
        if want.peek() == Some(&&c) {
            want.next();
        }
    }
    want.peek().is_none()
}

/// The penalty of skipping `chars` characters.
fn gap(chars: usize) -> i32 {
    i32::try_from(chars).unwrap_or(i32::MAX / 4) * GAP
}

fn bonus(starts: &[bool], at: usize) -> i32 {
    if starts[at] { BOUNDARY } else { 0 }
}

/// Which characters of `name` start a word: the first, and any after a separator.
fn word_starts(name: &str) -> Vec<bool> {
    let mut prev_separator = true;
    name.chars()
        .map(|c| {
            let start = prev_separator;
            prev_separator = matches!(c, '-' | '_' | '.' | '/' | ':' | ' ');
            start
        })
        .collect()
}
