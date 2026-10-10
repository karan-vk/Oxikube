//! Ranking: exact, prefix, subsequence, smart case, word starts.

use super::{rank, texts};
use crate::search::fuzzy::FuzzyService;
use crate::search::fuzzy::score::{self, Shape};

const COMMANDS: [&str; 6] = [
    "Workload Autoscale Horizontal",
    "Autoscale Settings",
    "Workload Scale",
    "Pod Delete",
    "Pod Describe",
    "Node Drain",
];

#[test]
fn scale_ranks_workload_scale_above_autoscale() {
    let found = rank("scale", &COMMANDS);
    let order = texts(&found, &COMMANDS);
    assert_eq!(order[0], "Workload Scale", "{order:?}");
    assert!(order.contains(&"Autoscale Settings"));
    assert!(order.contains(&"Workload Autoscale Horizontal"));
}

#[test]
fn the_palette_query_with_a_category_word_finds_the_title() {
    let found = rank("pod del", &COMMANDS);
    assert_eq!(texts(&found, &COMMANDS)[0], "Pod Delete");
}

#[test]
fn an_exact_match_beats_a_prefix_beats_a_subsequence() {
    let candidates = ["Podium Pods", "Pods", "Pod Delete", "Pending Of Dogs"];
    let found = rank("pods", &candidates);
    let order = texts(&found, &candidates);
    assert_eq!(order[0], "Pods", "exact: {order:?}");
    assert_eq!(order[1], "Podium Pods", "prefix: {order:?}");
    assert!(
        !order.contains(&"Pod Delete"),
        "'pods' is not a subsequence of it"
    );
}

#[test]
fn a_prefix_beats_the_same_letters_inside_a_word() {
    let candidates = ["Autopod", "Pod Delete"];
    let found = rank("pod", &candidates);
    assert_eq!(texts(&found, &candidates), ["Pod Delete", "Autopod"]);
}

#[test]
fn subsequences_match_in_order_only() {
    let candidates = ["Rollout Undo", "Undo Rollout"];
    let found = rank("rou", &candidates);
    assert_eq!(texts(&found, &candidates)[0], "Rollout Undo");
    assert!(rank("zzz", &candidates).is_empty());
    assert!(rank("odnu", &candidates).is_empty(), "order matters");
}

#[test]
fn case_is_smart() {
    let candidates = ["kube-system", "Kube", "KUBE"];
    // Lower case: any case matches.
    assert_eq!(rank("kube", &candidates).len(), 3);
    // A capital makes the match case-sensitive.
    let found = rank("Kube", &candidates);
    assert_eq!(texts(&found, &candidates), ["Kube"]);
}

#[test]
fn every_word_of_the_query_must_match_in_any_order() {
    let candidates = ["kube-system coredns", "default nginx", "Kube"];
    let found = rank("dns kube", &candidates);
    assert_eq!(texts(&found, &candidates), ["kube-system coredns"]);
}

#[test]
fn the_query_is_plain_text() {
    let candidates = ["Pod Delete", "Delete Pod", "Pod"];
    // nucleo's own syntax (`^` prefix, `!` negation, `$` suffix, `'` substring) is off: these
    // characters are not in any candidate, so nothing matches.
    assert!(rank("^pod", &candidates).is_empty());
    assert!(rank("!pod", &candidates).is_empty());
    assert!(rank("pod$", &candidates).is_empty());
}

#[test]
fn a_blank_query_keeps_every_candidate_in_order() {
    let candidates = ["b", "a", "c"];
    let found = rank("   ", &candidates);
    assert_eq!(texts(&found, &candidates), ["b", "a", "c"]);
    assert!(found.iter().all(|m| m.score == 0 && m.positions.is_empty()));
}

#[test]
fn the_limit_keeps_the_best() {
    let service = FuzzyService::new();
    let all = service.rank("o", &COMMANDS, usize::MAX);
    for limit in [0, 1, 2, all.len(), all.len() + 5] {
        let limited = service.rank("o", &COMMANDS, limit);
        let expected: Vec<_> = all.iter().take(limit).cloned().collect();
        assert_eq!(limited, expected, "limit {limit}");
    }
}

#[test]
fn scores_are_the_fuzzy_score_plus_the_documented_boosts() {
    assert_eq!(score::combine(100, Shape::Fuzzy, None), 100);
    assert_eq!(
        score::combine(100, Shape::Prefix, None),
        100 + score::PREFIX_BONUS
    );
    assert_eq!(
        score::combine(100, Shape::Exact, Some(0)),
        100 + score::EXACT_BONUS + score::RECENT_BONUS
    );
    // The recents boost fades with age and stops at nothing.
    assert!(score::recent_bonus(Some(0)) > score::recent_bonus(Some(3)));
    assert_eq!(score::recent_bonus(Some(10_000)), 0);
    assert_eq!(score::recent_bonus(None), 0);
}

#[test]
fn shapes_follow_case_rules() {
    assert_eq!(score::shape("Pods", "pods", false), Shape::Exact);
    assert_eq!(score::shape("Pods", "Pods", true), Shape::Exact);
    assert_eq!(score::shape("Pods", "pods", true), Shape::Fuzzy);
    assert_eq!(score::shape("Pod Delete", "pod", false), Shape::Prefix);
    assert_eq!(score::shape("Autopod", "pod", false), Shape::Fuzzy);
    assert_eq!(score::shape("Po", "pod", false), Shape::Fuzzy);
}
