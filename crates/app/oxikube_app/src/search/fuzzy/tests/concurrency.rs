//! One shared service, many callers: matchers are per call, so results never mix.

use std::sync::Arc;
use std::thread;

use crate::search::fuzzy::{FuzzyService, QueryGeneration};

fn titles() -> Vec<String> {
    (0..400)
        .map(|i| format!("Pod Verb{} object-{i}", i % 13))
        .collect()
}

#[test]
fn concurrent_queries_give_the_results_of_running_alone() {
    let service = Arc::new(FuzzyService::new());
    let titles = Arc::new(titles());
    let queries = [
        "pod", "verb3", "obj 1", "p v 7", "zzz", "o-39", "Pod", "d v1",
    ];
    let alone: Vec<_> = queries
        .iter()
        .map(|q| FuzzyService::new().rank(q, titles.as_slice(), 25))
        .collect();

    let workers: Vec<_> = (0..8)
        .map(|worker| {
            let (service, titles) = (service.clone(), titles.clone());
            thread::spawn(move || {
                let mut out = Vec::new();
                for round in 0..40 {
                    let q = (worker + round) % queries.len();
                    out.push((q, service.rank(queries[q], titles.as_slice(), 25)));
                }
                out
            })
        })
        .collect();
    for worker in workers {
        for (q, found) in worker.join().expect("a worker panicked") {
            assert_eq!(found, alone[q], "query {:?}", queries[q]);
        }
    }
}

#[test]
fn matchers_are_reused_between_calls() {
    let service = FuzzyService::new();
    let titles = titles();
    for _ in 0..20 {
        service.rank("pod", titles.as_slice(), 10);
    }
    // One caller at a time keeps one matcher: nothing is made per keystroke.
    assert!(
        format!("{service:?}").contains("idle_matchers: 1"),
        "{service:?}"
    );
}

#[test]
fn a_matcher_survives_a_panicking_caller() {
    let service = FuzzyService::new();
    let titles = titles();
    let before = service.rank("pod", titles.as_slice(), 10);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        service.rank_with(
            "pod",
            &[0usize],
            10,
            |_| -> &str { panic!("a bad text accessor") },
            |_| None,
        )
    }));
    assert!(panicked.is_err());
    assert_eq!(service.rank("pod", titles.as_slice(), 10), before);
}

#[test]
fn only_the_newest_query_may_write_its_result() {
    let mut generation = QueryGeneration::default();
    let first = generation.next();
    let second = generation.next();
    // Both are matching; the first finishes last.
    assert!(generation.is_current(second));
    assert!(!generation.is_current(first), "the stale result is dropped");
    assert_eq!(generation.current(), second);
}

#[test]
fn two_rapid_queries_on_threads_drop_the_stale_result() {
    let service = Arc::new(FuzzyService::new());
    let titles = Arc::new(titles());
    let mut generation = QueryGeneration::default();
    let mut shown: Vec<usize> = Vec::new();
    let queries = ["p", "po"];
    let handles: Vec<_> = queries
        .iter()
        .map(|query| {
            let generation = generation.next();
            let (service, titles) = (service.clone(), titles.clone());
            let query = query.to_string();
            thread::spawn(move || {
                (
                    generation,
                    service.rank(&query, titles.as_slice(), usize::MAX).len(),
                )
            })
        })
        .collect();
    // Results arrive in any order; only the newest is shown.
    for handle in handles {
        let (made_for, count) = handle.join().unwrap();
        if generation.is_current(made_for) {
            shown.push(count);
        }
    }
    let expected = FuzzyService::new()
        .rank("po", titles.as_slice(), usize::MAX)
        .len();
    assert_eq!(shown, [expected]);
}
