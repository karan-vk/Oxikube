//! The `/` filter over release rows.

use oxikube_domain::helm::{Release, ReleaseStatus};

use super::release;
use crate::helm_releases::{FilterProblem, ReleaseFilter};

fn rows() -> Vec<Release> {
    vec![
        release("api", ReleaseStatus::Deployed),
        release("worker", ReleaseStatus::Failed),
        release("cache", ReleaseStatus::PendingUpgrade),
    ]
}

fn names(filter: &ReleaseFilter, rows: &[Release]) -> Vec<String> {
    filter
        .apply(rows)
        .into_iter()
        .map(|ix| rows[ix].release.name.clone())
        .collect()
}

#[test]
fn empty_filter_lets_everything_through() {
    let rows = rows();
    let filter = ReleaseFilter::parse("");
    assert!(filter.is_empty());
    assert_eq!(names(&filter, &rows), ["api", "worker", "cache"]);
}

#[test]
fn substring_and_status_and_regex_match() {
    let rows = rows();
    assert_eq!(names(&ReleaseFilter::parse("wor"), &rows), ["worker"]);
    assert_eq!(names(&ReleaseFilter::parse("failed"), &rows), ["worker"]);
    assert_eq!(names(&ReleaseFilter::parse("pending"), &rows), ["cache"]);
    assert_eq!(
        names(&ReleaseFilter::parse("^(api|cache)$"), &rows),
        ["api", "cache"]
    );
}

#[test]
fn inverse_excludes_a_release_when_any_field_matches() {
    let rows = rows();
    assert_eq!(
        names(&ReleaseFilter::parse("!failed"), &rows),
        ["api", "cache"]
    );
}

#[test]
fn fuzzy_matches_a_subsequence() {
    let rows = rows();
    assert_eq!(names(&ReleaseFilter::parse("-f wkr"), &rows), ["worker"]);
}

#[test]
fn unusable_filters_say_why_and_filter_nothing() {
    let rows = rows();
    let bad = ReleaseFilter::parse("(");
    assert!(matches!(bad.problem(), Some(FilterProblem::Invalid(_))));
    assert_eq!(names(&bad, &rows).len(), 3);
    let labels = ReleaseFilter::parse("-l app=x");
    assert!(matches!(
        labels.problem(),
        Some(FilterProblem::Unsupported(_))
    ));
}
