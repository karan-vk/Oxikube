//! Property tests: random delta sequences keep every sorted index equal to a full re-sort of
//! the cache, and a consumer replaying the deltas ends with the same rows.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxikube_ports::Delta;
use proptest::prelude::*;

use super::*;
use crate::store::delta::RowOp;
use crate::store::index::SortedIndex;
use crate::store::{LabelSelector, SortField, SortKey, StoreFilter};

/// One random edit: (namespace, name, label value, creation second); `None` label = delete.
type Edit = (u8, u8, Option<u8>, u8);

fn object(ns: u8, name: u8, label: u8, created: u8, rv: usize) -> Resource {
    let mut r = pod()
        .namespace(format!("ns{ns}"))
        .name(format!("p{name:02}"))
        .label("tier", format!("t{label}"))
        .created(format!("2026-01-01T00:00:{:02}Z", created % 60))
        .build();
    r.meta.resource_version = Some(rv.to_string().into());
    r
}

fn sort_key(choice: u8) -> SortKey {
    let key = match choice % 4 {
        0 => SortKey::by(SortField::Namespace),
        1 => SortKey::by(SortField::Name),
        2 => SortKey::by(SortField::Created),
        _ => SortKey::by(SortField::Label("tier".into())),
    };
    if choice >= 4 { key.descending() } else { key }
}

fn filter(choice: u8) -> StoreFilter {
    match choice % 3 {
        0 => StoreFilter::default(),
        1 => StoreFilter::labels(LabelSelector::parse("tier in (t0,t1)").unwrap()),
        _ => StoreFilter::text("p0"),
    }
}

/// The expected rows: the model's objects that pass `filter`, fully sorted.
fn expected(
    model: &BTreeMap<(u8, u8), Resource>,
    filter: &StoreFilter,
    sort: &SortKey,
) -> Vec<String> {
    let mut objs: Vec<Arc<StoreObject>> = model
        .values()
        .map(|r| Arc::new(StoreObject::Resource(r.clone())))
        .filter(|o| filter.matches(o))
        .collect();
    objs.sort_by(|a, b| {
        let ord = sort
            .value_of(a)
            .cmp(&sort.value_of(b))
            .then_with(|| a.key().cmp(&b.key()));
        if sort.descending { ord.reverse() } else { ord }
    });
    names(&objs)
}

fn edits() -> impl Strategy<Value = Vec<Vec<Edit>>> {
    let edit = (
        0u8..3,
        0u8..30,
        proptest::option::weighted(0.75, 0u8..3),
        0u8..60,
    );
    proptest::collection::vec(proptest::collection::vec(edit, 1..80), 1..12)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// The index alone: incremental ops replayed on a mirror equal the index's own snapshot,
    /// and both equal a full re-sort.
    #[test]
    fn sorted_index_matches_a_full_resort(batches in edits(), sort in 0u8..8, f in 0u8..3) {
        let (sort, filter) = (sort_key(sort), filter(f));
        let mut index = SortedIndex::new(filter.clone(), sort.clone());
        let mut model = BTreeMap::new();
        let mut mirror: Vec<Arc<StoreObject>> = Vec::new();
        let mut rv = 0;
        for batch in batches {
            // A batch's net change, one entry per key (what `ObjectCache::apply` hands over).
            let mut net: BTreeMap<(u8, u8), Option<Resource>> = BTreeMap::new();
            for (ns, name, label, created) in batch {
                rv += 1;
                let state = label.map(|label| object(ns, name, label, created, rv));
                match &state {
                    Some(r) => model.insert((ns, name), r.clone()),
                    None => model.remove(&(ns, name)),
                };
                net.insert((ns, name), state);
            }
            let mut removed = Vec::new();
            let mut upserted = Vec::new();
            for ((ns, name), state) in net {
                match state {
                    Some(r) => upserted.push(Arc::new(StoreObject::Resource(r))),
                    None => removed.push(crate::store::ObjectKey::new(
                        Some(&format!("ns{ns}")),
                        &format!("p{name:02}"),
                    )),
                }
            }
            let mut ops: Vec<RowOp> = Vec::new();
            index.apply(&removed, &upserted, Some(&mut ops));
            ops.iter().for_each(|op| op.apply(&mut mirror));
            let want = expected(&model, &filter, &sort);
            prop_assert_eq!(names(&index.snapshot()), want.clone());
            prop_assert_eq!(names(&mirror), want);
        }
    }

    /// The whole pipeline: random batches through a scripted feed; the consumer's mirror (built
    /// only from the stream's snapshots and ops) always equals a full re-sort of the model.
    #[test]
    fn subscription_stream_matches_a_full_resort(batches in edits(), sort in 0u8..8, f in 0u8..3) {
        let (sort, filter) = (sort_key(sort), filter(f));
        let mut model = BTreeMap::new();
        let mut rv = 0;
        let mut feed = vec![batch(vec![Delta::Restarted(vec![])])];
        let mut states = Vec::new();
        for edits in batches {
            let mut deltas = Vec::new();
            for (ns, name, label, created) in edits {
                rv += 1;
                match label {
                    Some(label) => {
                        let r = object(ns, name, label, created, rv);
                        model.insert((ns, name), r.clone());
                        deltas.push(Delta::Applied(r));
                    }
                    None => {
                        // Deleting an unknown object is legal (out-of-order feeds).
                        let r = model.remove(&(ns, name)).unwrap_or_else(|| object(ns, name, 0, 0, rv));
                        deltas.push(Delta::Deleted(r));
                    }
                }
            }
            feed.push(batch(deltas));
            states.push(expected(&model, &filter, &sort));
        }
        let mut h = Harness::new();
        h.resources.script().watch.push_ok(timeline(feed));
        let mut sub = h.subscribe(all(pods()).with_filter(filter).with_sort(sort));
        let mut m = Mirror::default();
        m.drain(&mut sub);
        for want in states {
            h.advance(1);
            m.drain(&mut sub);
            prop_assert_eq!(m.names(), want);
        }
    }
}
