//! The pure admission and selection rules.

use std::time::Duration;

use oxikube_domain::session::WatchScope;
use oxikube_ports::FeedVariant;

use crate::budget::BudgetConfig;
use crate::budget::policy::{Admission, Breach, IdleFeed, Usage, admit, plan};

fn limits() -> BudgetConfig {
    BudgetConfig {
        max_feeds: 4,
        max_objects: 100,
        metadata_above: 60,
        idle_grace: Duration::from_secs(30),
    }
}

fn usage(feeds: usize, objects: u64) -> Usage {
    Usage { feeds, objects }
}

fn idle(id: u64, objects: u64) -> IdleFeed {
    IdleFeed { id, objects }
}

fn open(variant: FeedVariant, evict: &[u64]) -> Admission {
    Admission::Open {
        variant,
        evict: evict.to_vec(),
    }
}

#[test]
fn under_every_limit_the_request_is_granted_as_asked() {
    for variant in [FeedVariant::Full, FeedVariant::Metadata, FeedVariant::Table] {
        assert_eq!(
            admit(&limits(), usage(1, 10), &[], variant),
            open(variant, &[])
        );
    }
}

#[test]
fn above_the_metadata_threshold_a_full_request_degrades() {
    assert_eq!(
        admit(&limits(), usage(1, 60), &[], FeedVariant::Full),
        open(FeedVariant::Metadata, &[])
    );
    // Other variants are already the cheap ones.
    assert_eq!(
        admit(&limits(), usage(1, 60), &[], FeedVariant::Table),
        open(FeedVariant::Table, &[])
    );
}

#[test]
fn at_the_caps_requests_are_refused() {
    assert_eq!(
        admit(&limits(), usage(4, 0), &[], FeedVariant::Metadata),
        Admission::Refuse(Breach::Feeds { open: 4, max: 4 })
    );
    assert_eq!(
        admit(&limits(), usage(1, 100), &[], FeedVariant::Full),
        Admission::Refuse(Breach::Objects {
            held: 100,
            max: 100
        })
    );
}

#[test]
fn idle_feeds_are_evicted_oldest_first_to_get_under_a_cap() {
    let three = [idle(7, 5), idle(3, 5), idle(9, 5)];
    assert_eq!(
        admit(&limits(), usage(4, 15), &three, FeedVariant::Table),
        open(FeedVariant::Table, &[7])
    );
    assert_eq!(
        admit(
            &limits(),
            usage(2, 110),
            &[idle(1, 6), idle(2, 30)],
            FeedVariant::Table
        ),
        open(FeedVariant::Table, &[1, 2])
    );
}

#[test]
fn a_refusal_evicts_nothing() {
    let one = [idle(1, 1)];
    assert_eq!(
        admit(&limits(), usage(1, 200), &one, FeedVariant::Full),
        Admission::Refuse(Breach::Objects {
            held: 199,
            max: 100
        })
    );
}

#[test]
fn idle_feeds_are_evicted_to_avoid_a_degrade_only_when_that_is_enough() {
    // Dropping the idle feeds gets under the threshold: full, with evictions.
    assert_eq!(
        admit(
            &limits(),
            usage(3, 70),
            &[idle(1, 5), idle(2, 10)],
            FeedVariant::Full
        ),
        open(FeedVariant::Full, &[1, 2])
    );
    // It would not: degrade, and leave the idle feeds alone.
    assert_eq!(
        admit(
            &limits(),
            usage(3, 90),
            &[idle(1, 5), idle(2, 10)],
            FeedVariant::Full
        ),
        open(FeedVariant::Metadata, &[])
    );
}

#[test]
fn breach_reasons_say_which_limit_and_what_to_do() {
    let feeds = Breach::Feeds { open: 4, max: 4 }.reason("full v1/Pod in a");
    assert!(
        feeds.contains("4 of 4 feeds") && feeds.contains("v1/Pod"),
        "{feeds}"
    );
    let objects = Breach::Objects {
        held: 120,
        max: 100,
    }
    .reason("table v1/Pod cluster-wide");
    assert!(objects.contains("120 objects (limit 100)"), "{objects}");
    assert!(
        objects.contains("narrow the namespace selection"),
        "{objects}"
    );
}

fn some(names: &[&str]) -> Vec<Option<String>> {
    names.iter().map(|n| Some((*n).to_owned())).collect()
}

fn namespaces(names: &[&str]) -> WatchScope {
    WatchScope::Namespaces(names.iter().map(|n| (*n).to_owned()).collect())
}

#[test]
fn a_selection_change_starts_keeps_and_stops_by_namespace() {
    let change = plan(&some(&["a", "b"]), &namespaces(&["b", "c"]));
    assert_eq!(change.start, some(&["c"]));
    assert_eq!(change.keep, some(&["b"]));
    assert_eq!(change.stop, some(&["a"]));
}

#[test]
fn all_and_sets_swap_the_cluster_wide_feed() {
    let change = plan(&some(&["a", "b"]), &WatchScope::Cluster);
    assert_eq!((change.start, change.stop), (vec![None], some(&["a", "b"])));
    let back = plan(&[None], &namespaces(&["a"]));
    assert_eq!((back.start, back.stop), (some(&["a"]), vec![None]));
    assert!(plan(&[None], &WatchScope::Cluster).is_empty());
    let fresh = plan(&[], &namespaces(&["b", "a"]));
    assert_eq!(fresh.start, some(&["a", "b"]));
}
