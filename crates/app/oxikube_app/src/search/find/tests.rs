//! The find rule, the scan and the navigator.

use super::{
    FindMatches, FindNavigator, FindQuery, MAX_MATCHES, find_in_text, next_match, previous_match,
};
use crate::search::filter::FilterError;

fn query(text: &str) -> FindQuery {
    FindQuery::new(text).expect("compiles").expect("not empty")
}

fn ranges_in(query: &FindQuery, line: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    query.ranges_in(line, &mut out);
    out
}

/// A navigator over the start of every match of a scan, as the detail builds it.
fn navigator(found: &FindMatches) -> FindNavigator {
    let mut nav = FindNavigator::new();
    nav.set(
        found.ranges.iter().map(|r| r.start as u64).collect(),
        found.truncated,
    );
    nav
}

#[test]
fn an_empty_query_finds_nothing_and_a_bad_one_is_an_error() {
    assert!(FindQuery::new("").expect("empty is fine").is_none());
    assert!(matches!(FindQuery::new("a("), Err(FilterError::Regex(_))));
    assert!(matches!(
        FindQuery::new(&"a".repeat(600)),
        Err(FilterError::TooLong(_))
    ));
}

#[test]
fn text_matches_as_a_case_insensitive_substring_and_regex_syntax_as_a_regex() {
    let regex = query("a.b");
    assert_eq!(ranges_in(&regex, "xA-Bx"), [1..4], "a regex dot");
    assert_eq!(ranges_in(&regex, "xa.bx"), [1..4]);
    assert_eq!(ranges_in(&query("web"), "web WEB"), [0..3, 4..7]);
    assert_eq!(ranges_in(&query("pod-1"), "my-pod-1"), [3..8]);
}

#[test]
fn find_in_text_reports_ranges_into_the_whole_text() {
    let text = "name: web\nlabels:\n  app: web\n";
    let found = find_in_text(text, &query("web"));
    assert_eq!(found.ranges, [6..9, 25..28]);
    for r in &found.ranges {
        assert_eq!(&text[r.clone()], "web");
    }
    assert!(!found.truncated);
    assert!(find_in_text(text, &query("zzz")).is_empty());
}

#[test]
fn a_scan_is_capped() {
    let text = "a".repeat(MAX_MATCHES * 3);
    let found = find_in_text(&text, &query("a"));
    assert_eq!(found.len(), MAX_MATCHES);
    assert!(found.truncated);
}

#[test]
fn next_and_previous_wrap_around() {
    let list: &[u64] = &[10, 20, 30];
    // From nothing: next starts at the anchor, previous at the last.
    assert_eq!(next_match(list, None, 15), Some(20));
    assert_eq!(next_match(list, None, 20), Some(20));
    assert_eq!(next_match(list, None, 99), Some(10), "nothing after: wrap");
    assert_eq!(previous_match(list, None), Some(30));
    // Walking forward wraps from the last to the first.
    assert_eq!(next_match(list, Some(10), 0), Some(20));
    assert_eq!(next_match(list, Some(20), 0), Some(30));
    assert_eq!(next_match(list, Some(30), 0), Some(10));
    // And backward from the first to the last.
    assert_eq!(previous_match(list, Some(30)), Some(20));
    assert_eq!(previous_match(list, Some(10)), Some(30));
    // A current match that is gone is skipped by position.
    assert_eq!(next_match(list, Some(15), 0), Some(20));
    assert_eq!(previous_match(list, Some(15)), Some(10));
    // No matches: nowhere to go.
    let none: &[u64] = &[];
    assert_eq!(next_match(none, None, 0), None);
    assert_eq!(previous_match(none, Some(3)), None);
}

#[test]
fn n_wraps_at_the_last_match() {
    let mut nav = FindNavigator::new();
    nav.set(vec![0, 2, 4], false);
    assert_eq!(nav.len(), 3);
    assert_eq!(nav.position(), None, "none is current until n is pressed");

    assert_eq!(
        nav.next(1),
        Some(1),
        "from the top of the view: the first match after it"
    );
    assert_eq!(nav.current(), Some(2));
    assert_eq!(nav.next(0), Some(2));
    assert_eq!(
        nav.next(0),
        Some(0),
        "n at the last match wraps to the first"
    );
    assert_eq!(nav.position(), Some((1, 3)));
    assert_eq!(nav.previous(), Some(2), "N at the first wraps to the last");
    assert_eq!(nav.position(), Some((3, 3)));
    assert_eq!(nav.previous(), Some(1));
}

#[test]
fn n_and_shift_n_on_a_text_with_two_matches_per_line_stop_at_each() {
    let text = "web web\nx\nweb\n";
    let mut nav = navigator(&find_in_text(text, &query("web")));
    let stops: Vec<usize> = (0..4).filter_map(|_| nav.next(0)).collect();
    assert_eq!(stops, [0, 1, 2, 0], "three stops, then the wrap");
}

#[test]
fn replacing_the_matches_keeps_the_current_one_when_it_survives() {
    let mut nav = FindNavigator::new();
    nav.set(vec![5, 10, 20], false);
    nav.next(0);
    nav.next(0);
    assert_eq!(nav.current(), Some(10));
    nav.set(vec![10, 20, 30], false);
    assert_eq!(nav.current(), Some(10), "still a match");
    nav.set(vec![7], true);
    assert_eq!(nav.current(), None);
    assert!(nav.truncated());
    nav.clear();
    assert!(nav.is_empty());
}

#[test]
fn a_five_megabyte_scan_is_bounded_and_off_the_ui_threads_budget_even_in_debug() {
    // The detail scans a YAML text on the background executor; this is the cost of one scan.
    let line =
        "    service.route: upstream=payments.svc.cluster.local:8080 timeout=30s retries=3\n";
    let text = line.repeat(5 * 1024 * 1024 / line.len());
    let q = query("timeout=30s");
    let started = std::time::Instant::now();
    let found = find_in_text(&text, &q);
    let elapsed = started.elapsed();
    eprintln!(
        "find_in_text: {} bytes, {} matches in {elapsed:?}",
        text.len(),
        found.len()
    );
    assert_eq!(found.len(), MAX_MATCHES);
    assert!(found.truncated);
    assert!(elapsed < std::time::Duration::from_secs(2), "{elapsed:?}");
    // The worst case for the scan: no match, so every byte is read.
    let started = std::time::Instant::now();
    assert!(find_in_text(&text, &query("no such text")).is_empty());
    let elapsed = started.elapsed();
    eprintln!(
        "find_in_text: no match over {} bytes in {elapsed:?}",
        text.len()
    );
    assert!(elapsed < std::time::Duration::from_secs(2), "{elapsed:?}");
}
