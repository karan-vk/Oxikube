//! The `-s` status filter (E12-S07): its syntax, that it stands alone, and what it matches.

use oxikube_domain::view::Bucket;
use oxikube_testkit::pod;

use super::*;
use crate::store::filter::FilterError;
use crate::store::{StoreFilter, StoreObject};

#[test]
fn a_status_word_makes_a_store_filter() {
    for (input, bucket) in [
        ("-s pending", Bucket::Pending),
        ("/-s Failed", Bucket::Failed),
        ("-s not-ready", Bucket::NotReady),
        ("-s  notready ", Bucket::NotReady),
    ] {
        assert_eq!(ok(input), FilterExpr::Status(bucket), "{input:?}");
        let parts = parts(input);
        assert_eq!(parts.filter.status, Some(bucket));
        assert!(
            parts.selector.is_none(),
            "applied by the store, not the server"
        );
        assert!(!parts.is_empty());
    }
}

#[test]
fn a_bare_flag_is_a_filter_being_typed() {
    assert_eq!(ok("-s"), FilterExpr::Empty);
    assert!(parts("/-s ").is_empty());
}

#[test]
fn an_unknown_word_or_a_combination_is_an_error_that_keeps_the_last_filter() {
    assert_eq!(
        parse("-s sleepy"),
        Err(FilterError::UnknownStatus("sleepy".to_owned()))
    );
    assert_eq!(parse("!-s failed"), Err(FilterError::InvertedStatus));
    assert!(parse("web -s failed").is_ok_and(|e| matches!(e, FilterExpr::Text(_))));
    assert!(parse("-s failed -l app=x").is_err());
}

#[test]
fn it_matches_the_objects_in_the_bucket() {
    let failed = StoreObject::Resource(pod().name("a").failed().build());
    let running = StoreObject::Resource(pod().name("b").running().build());
    let filter = parts("-s failed").filter;
    assert!(filter.matches(&failed));
    assert!(!filter.matches(&running));
    assert!(StoreFilter::default().matches(&failed));
}

#[test]
fn a_status_change_recomputes_rather_than_narrows() {
    let pending = parts("-s pending").filter;
    let failed = parts("-s failed").filter;
    assert!(!failed.narrows(&pending));
    assert!(
        !pending.narrows(&StoreFilter::default()),
        "a new restriction recomputes"
    );
}
