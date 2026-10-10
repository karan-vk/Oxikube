//! Ties, recents and stability: the order is total, so results never flicker.

use super::texts;
use crate::search::fuzzy::FuzzyService;

#[test]
fn equal_scores_break_by_recents_then_alphabetically() {
    let candidates = ["Pod Zebra", "Pod Alpha", "Pod Mango", "Pod Beta"];
    let service = FuzzyService::new();

    let plain = service.rank("pod", &candidates, usize::MAX);
    assert_eq!(
        texts(&plain, &candidates),
        ["Pod Alpha", "Pod Beta", "Pod Mango", "Pod Zebra"],
        "alphabetical"
    );

    // Mango was run last, Zebra before it.
    let recents = [Some(1), None, Some(0), None].map(|r: Option<usize>| r);
    let ranked = service.rank_with(
        "pod",
        &[0usize, 1, 2, 3],
        usize::MAX,
        |ix| candidates[*ix],
        |ix| recents[*ix],
    );
    let order: Vec<_> = ranked.iter().map(|m| candidates[m.index]).collect();
    assert_eq!(order, ["Pod Mango", "Pod Zebra", "Pod Alpha", "Pod Beta"]);
}

#[test]
fn the_order_does_not_depend_on_the_input_order() {
    let mut candidates = vec![
        "Pod Zebra",
        "Pod Alpha",
        "Pod Mango",
        "Pod Beta",
        "Pod Alpha Two",
        "Workload Pod Scale",
    ];
    let service = FuzzyService::new();
    let first: Vec<_> = {
        let found = service.rank("pod", &candidates, usize::MAX);
        texts(&found, &candidates)
            .into_iter()
            .map(String::from)
            .collect()
    };
    for _ in 0..candidates.len() {
        candidates.rotate_left(1);
        let found = service.rank("pod", &candidates, usize::MAX);
        let again: Vec<_> = texts(&found, &candidates)
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(again, first);
    }
}

#[test]
fn the_same_input_gives_the_same_output_every_time() {
    let candidates: Vec<String> = (0..300).map(|i| format!("Pod Item {}", i % 17)).collect();
    let service = FuzzyService::new();
    let first = service.rank("pod it", &candidates, 50);
    for _ in 0..5 {
        assert_eq!(service.rank("pod it", &candidates, 50), first);
    }
}

#[test]
fn a_blank_query_lists_the_recent_ones_first_then_the_rest_in_order() {
    let candidates = ["a", "b", "c", "d", "e"];
    let recency = |ix: &usize| match *ix {
        3 => Some(0),
        1 => Some(1),
        _ => None,
    };
    let found = FuzzyService::new().rank_with(
        "",
        &[0usize, 1, 2, 3, 4],
        usize::MAX,
        |ix| candidates[*ix],
        recency,
    );
    let order: Vec<_> = found.iter().map(|m| candidates[m.index]).collect();
    assert_eq!(order, ["d", "b", "a", "c", "e"]);
    let limited =
        FuzzyService::new().rank_with("", &[0usize, 1, 2, 3, 4], 3, |ix| candidates[*ix], recency);
    assert_eq!(limited.len(), 3);
    assert_eq!(limited[0].index, 3);
}

#[test]
fn a_blank_query_leads_with_recents_even_when_they_are_ranked_past_the_bonus() {
    // The global recency ranks run to 50, but the bonus is spent by rank 12: a recent
    // candidate ranked 20th or 30th still leads a blank-query list.
    let candidates = ["a", "b", "c", "d"];
    let recency = |ix: &usize| match *ix {
        2 => Some(20),
        3 => Some(30),
        _ => None,
    };
    let found = FuzzyService::new().rank_with(
        "",
        &[0usize, 1, 2, 3],
        usize::MAX,
        |ix| candidates[*ix],
        recency,
    );
    let order: Vec<_> = found.iter().map(|m| candidates[m.index]).collect();
    assert_eq!(order, ["c", "d", "a", "b"]);
}

#[test]
fn a_recent_candidate_does_not_beat_a_much_better_match() {
    let candidates = ["Cluster Toggle Read-only Mode Setting", "Toggle Read-only"];
    let found = FuzzyService::new().rank_with(
        "toggle read-only",
        &[0usize, 1],
        usize::MAX,
        |ix| candidates[*ix],
        // The long one is the latest command run.
        |ix| (*ix == 0).then_some(0),
    );
    assert_eq!(candidates[found[0].index], "Toggle Read-only");
}

#[test]
fn recents_decide_between_near_equal_matches() {
    let candidates = ["Pod Describe", "Pod Delete"];
    let ranked = |recent: usize| {
        let found = FuzzyService::new().rank_with(
            "pod de",
            &[0usize, 1],
            usize::MAX,
            |ix| candidates[*ix],
            |ix| (*ix == recent).then_some(0),
        );
        candidates[found[0].index]
    };
    assert_eq!(ranked(0), "Pod Describe");
    assert_eq!(ranked(1), "Pod Delete");
}
