//! How a candidate's final score is made: the weights, in one place.
//!
//! `nucleo-matcher` scores a match from its alignment (about 16 per matched character, plus
//! bonuses for word starts and consecutive characters). The boosts below sit on top of that and
//! are deliberately **small**: a better alignment always wins, and the boosts decide between
//! candidates that are about as good. Changing a weight changes the ranking tests, which is the
//! review.

use std::cmp::Ordering;

/// The candidate's text is the query, apart from case: `pods` finds `Pods` first.
pub const EXACT_BONUS: u32 = 64;

/// The candidate's text starts with the query, apart from case: `pod` finds `Pod Delete` before
/// `Autopod`.
pub const PREFIX_BONUS: u32 = 32;

/// What the most recent candidate gets; the next one gets [`RECENT_STEP`] less, and so on down to
/// nothing. About three quarters of one matched character: it decides between near-equal matches
/// without lifting a weak match over a strong one.
pub const RECENT_BONUS: u32 = 12;

/// How much less each older recent candidate gets.
pub const RECENT_STEP: u32 = 1;

/// How the candidate's text relates to the whole query, the largest boost that applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The text is the query (case aside).
    Exact,
    /// The text starts with the query (case aside).
    Prefix,
    /// Neither: a plain subsequence match.
    Fuzzy,
}

/// The score of a candidate: the fuzzy `score` of its alignment plus the boost of its `shape`
/// and of its place among the recents (`recency`: 0 is the latest; `None` for a candidate that
/// was not used lately).
pub fn combine(score: u32, shape: Shape, recency: Option<usize>) -> u32 {
    let shape = match shape {
        Shape::Exact => EXACT_BONUS,
        Shape::Prefix => PREFIX_BONUS,
        Shape::Fuzzy => 0,
    };
    score + shape + recent_bonus(recency)
}

/// The boost of the `recency`-th most recent candidate.
pub fn recent_bonus(recency: Option<usize>) -> u32 {
    let Some(rank) = recency else {
        return 0;
    };
    let lost = u32::try_from(rank)
        .unwrap_or(u32::MAX)
        .saturating_mul(RECENT_STEP);
    RECENT_BONUS.saturating_sub(lost)
}

/// Classifies `text` against `query` (already trimmed): `case_sensitive` when the query has a
/// capital, as in nucleo's smart case.
pub fn shape(text: &str, query: &str, case_sensitive: bool) -> Shape {
    if query.is_empty() {
        // A blank query has no shape.
        return Shape::Fuzzy;
    }
    let mut text = text.chars();
    for want in query.chars() {
        if !text
            .next()
            .is_some_and(|have| same(have, want, case_sensitive))
        {
            return Shape::Fuzzy;
        }
    }
    if text.next().is_none() {
        Shape::Exact
    } else {
        Shape::Prefix
    }
}

fn same(a: char, b: char, case_sensitive: bool) -> bool {
    a == b || (!case_sensitive && a.to_lowercase().eq(b.to_lowercase()))
}

/// Whether the query asks for case to matter: it has a capital (smart case).
pub fn is_case_sensitive(query: &str) -> bool {
    query.chars().any(char::is_uppercase)
}

/// `a` against `b`, alphabetical and case-insensitive; the tie-break between equal scores.
pub fn alphabetical(a: &str, b: &str) -> Ordering {
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}
