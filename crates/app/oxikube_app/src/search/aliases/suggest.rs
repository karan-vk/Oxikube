//! "Did you mean": close names for an unknown input, by prefix and by edit distance.
//!
//! Only runs for a name that is unknown, which is the rare path of a jump-bar keystroke (a prefix
//! of a longer alias is unknown until it is complete), and it scans the names once with a
//! bounded edit distance, so a cluster with hundreds of CRDs costs well under a millisecond.

use std::sync::Arc;

use super::index::Index;

/// The most suggestions [`Resolution::Unknown`](super::Resolution::Unknown) carries.
pub const MAX_SUGGESTIONS: usize = 5;

/// Longest name compared by edit distance; longer inputs only get prefix matches.
const MAX_COMPARED: usize = 48;

/// Close names for `input` (lower-case), best first: names that start with it (shortest first),
/// then names within a small edit distance (closest first, built-in and user names before
/// discovered ones, then alphabetical).
pub(super) fn suggest(index: &Index, input: &str) -> Vec<Arc<str>> {
    if input.is_empty() {
        return Vec::new();
    }
    let allowed = match input.len() {
        0..=2 => 0,
        3..=5 => 1,
        _ => 2,
    };
    let check_distance = allowed > 0 && input.len() <= MAX_COMPARED;

    // Sorts by (class, closeness, source, name): a prefix match is closer when shorter, an edit
    // match when fewer edits away.
    let mut found: Vec<(u8, usize, u8, &Arc<str>)> = Vec::new();
    for name in index.names() {
        let source = index.source_of(name).map_or(3, |s| s as u8);
        if name.starts_with(input) {
            found.push((0, name.len(), source, name));
        } else if check_distance && name.len() <= MAX_COMPARED {
            if let Some(distance) = bounded_distance(input.as_bytes(), name.as_bytes(), allowed) {
                found.push((1, distance, source, name));
            }
        }
    }
    found.sort();
    found
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|f| f.3.clone())
        .collect()
}

/// The Levenshtein distance of `a` and `b` when it is at most `max`, else `None`. Stops as soon
/// as a whole row of the table exceeds `max`.
fn bounded_distance(a: &[u8], b: &[u8], max: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        current[0] = i + 1;
        let mut row_min = current[0];
        for (j, &cb) in b.iter().enumerate() {
            let substitute = previous[j] + usize::from(ca != cb);
            current[j + 1] = substitute.min(previous[j + 1] + 1).min(current[j] + 1);
            row_min = row_min.min(current[j + 1]);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[b.len()];
    (distance <= max).then_some(distance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_is_bounded_levenshtein() {
        assert_eq!(bounded_distance(b"pods", b"pods", 2), Some(0));
        assert_eq!(bounded_distance(b"pdos", b"pods", 2), Some(2));
        assert_eq!(bounded_distance(b"depoy", b"deploy", 1), Some(1));
        assert_eq!(bounded_distance(b"abc", b"xyz", 2), None);
        assert_eq!(bounded_distance(b"a", b"abcdef", 2), None);
    }
}
