//! Tests of the fuzzy ranking service: ranking, unicode, ordering stability, concurrency and the
//! latency budget.

mod budget;
mod concurrency;
mod highlight;
mod ordering;
mod ranking;
mod unicode;

use super::{FuzzyService, Match};

/// The candidate texts of `found`, in rank order.
fn texts<'a>(found: &[Match], candidates: &[&'a str]) -> Vec<&'a str> {
    found.iter().map(|m| candidates[m.index]).collect()
}

/// Ranks `candidates` for `query` with a fresh service, no recents, no limit.
fn rank(query: &str, candidates: &[&str]) -> Vec<Match> {
    FuzzyService::new().rank(query, candidates, usize::MAX)
}
