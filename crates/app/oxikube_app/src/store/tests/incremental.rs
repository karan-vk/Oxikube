//! Typing more of a literal pattern filters the rows already shown, not the whole cache
//! (docs/PERFORMANCE.md rule 4): the number of rows examined says which.

use std::sync::Arc;

use super::*;
use crate::store::filter::parse;
use crate::store::index::SortedIndex;
use crate::store::{SortKey, StoreFilter};

fn filter(input: &str) -> StoreFilter {
    parse(input).expect("parses").parts().filter
}

/// `count` pods: `web-<i>` for every third, `db-<i>` for the rest.
fn cache(count: usize) -> Vec<Arc<StoreObject>> {
    (0..count)
        .map(|i| {
            let name = if i % 3 == 0 { "web" } else { "db" };
            let mut r = pod().namespace("x").name(&format!("{name}-{i}")).build();
            r.meta.resource_version = Some("1".into());
            Arc::new(StoreObject::Resource(r))
        })
        .collect()
}

#[test]
fn extending_a_literal_pattern_scans_the_previous_result_not_the_cache() {
    let objects = cache(3_000);
    let mut index = SortedIndex::new(filter("web"), SortKey::default());
    index.apply(&[], &objects, None);
    index.resort();
    assert_eq!(index.snapshot().len(), 1_000);

    // `web-1` extends `web`: only the 1000 rows that passed `web` are examined, not the 3000.
    let narrower = filter("web-1");
    assert!(narrower.narrows(&filter("web")));
    let scanned = index.narrow(narrower.clone(), SortKey::default());
    assert_eq!(scanned, 1_000, "the previous result set, not the cache");
    assert!(scanned < objects.len());

    // The result is what a fresh pass over the whole cache gives.
    let mut fresh = SortedIndex::new(narrower, SortKey::default());
    fresh.apply(&[], &objects, None);
    fresh.resort();
    let names = |i: &SortedIndex| -> Vec<String> {
        i.snapshot()
            .iter()
            .map(|o| format!("{:?}", o.key()))
            .collect()
    };
    assert_eq!(names(&index), names(&fresh));

    // And one more character examines only that smaller set.
    let scanned = index.narrow(filter("web-12"), SortKey::default());
    assert_eq!(scanned, fresh.snapshot().len());
}

#[test]
fn a_regex_is_not_derived_from_the_previous_result() {
    // Appending to a regex can widen it (`a` to `a|b`), so it is recomputed from the cache.
    assert!(!filter("web|db").narrows(&filter("web")));
    assert!(!filter("web.*1").narrows(&filter("web.*")));
}
