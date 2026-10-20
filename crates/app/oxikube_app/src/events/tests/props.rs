//! Property test: random insert / update / delete / capacity sequences keep the ring, the key
//! map and every index in agreement with a brute-force scan, and every query in agreement with a
//! naive model (a list scanned and sorted for each question).

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::event::{Event, EventType};
use proptest::prelude::*;

use super::{ev, warning};
use crate::events::key::seen_at;
use crate::events::{EventBuffer, EventFilter, EventKey};

#[derive(Debug, Clone)]
enum Op {
    Upsert {
        uid: u8,
        ns: u8,
        object: u8,
        secs: i64,
        warn: bool,
        count: u32,
    },
    Delete(u8),
    Capacity(usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        8 => (0u8..24, 0u8..3, 0u8..5, 0i64..60, any::<bool>(), 1u32..5).prop_map(
            |(uid, ns, object, secs, warn, count)| Op::Upsert { uid, ns, object, secs, warn, count }
        ),
        2 => (0u8..24).prop_map(Op::Delete),
        1 => (1usize..16).prop_map(Op::Capacity),
    ]
}

fn build(uid: u8, ns: u8, object: u8, secs: i64, warn: bool, count: u32) -> Event {
    let (uid, ns, object) = (format!("e{uid}"), format!("ns{ns}"), format!("o{object}"));
    let mut e = if warn {
        warning(&uid, &ns, &object, secs, "BackOff")
    } else {
        ev(&uid, &ns, &object, secs)
    };
    e.count = count;
    e
}

/// The naive model: (order, event) pairs in arrival order.
#[derive(Default)]
struct Model {
    capacity: usize,
    seq: u64,
    events: Vec<((Timestamp, u64), Event)>,
}

impl Model {
    fn upsert(&mut self, event: Event) {
        let key = EventKey::of(&event);
        if let Some(i) = self.events.iter().position(|(_, e)| EventKey::of(e) == key) {
            if self.events[i].1 == event {
                return;
            }
            self.events.remove(i);
        } else if self.events.len() >= self.capacity {
            self.seq += 1;
            let order = (seen_at(&event), self.seq);
            let oldest = self.oldest();
            if order < self.events[oldest].0 {
                return;
            }
            self.events.remove(oldest);
            self.events.push((order, event));
            return;
        }
        self.seq += 1;
        self.events.push(((seen_at(&event), self.seq), event));
    }

    fn oldest(&self) -> usize {
        (0..self.events.len())
            .min_by_key(|&i| self.events[i].0)
            .expect("a full model is not empty")
    }

    fn delete(&mut self, uid: &str) {
        self.events.retain(|(_, e)| e.uid.as_deref() != Some(uid));
    }

    fn capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        while self.events.len() > capacity {
            let oldest = self.oldest();
            self.events.remove(oldest);
        }
    }

    fn query(&self, filter: &EventFilter) -> Vec<Event> {
        let mut out: Vec<_> = self
            .events
            .iter()
            .filter(|(_, e)| filter.matches(e))
            .collect();
        out.sort_by_key(|(order, _)| std::cmp::Reverse(*order));
        out.into_iter().map(|(_, e)| e.clone()).collect()
    }
}

fn filters() -> Vec<EventFilter> {
    let since: Timestamp = super::t0() + jiff::SignedDuration::from_secs(30);
    vec![
        EventFilter::new(),
        EventFilter::warnings(),
        EventFilter::new().types([EventType::Normal]),
        EventFilter::new().namespaces(["ns1"]),
        EventFilter::new().namespaces(["ns0", "ns2"]),
        EventFilter::new().object("o3"),
        EventFilter::new().object("o2").types([EventType::Warning]),
        EventFilter::new().since(since),
        EventFilter::warnings().namespaces(["ns0"]).since(since),
        EventFilter::new().text("e1").expect("a substring"),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn indexes_and_queries_agree_with_a_naive_model(
        capacity in 1usize..12,
        ops in prop::collection::vec(op(), 1..120),
    ) {
        let mut buffer = EventBuffer::new(capacity);
        let mut model = Model { capacity, ..Model::default() };
        for op in ops {
            match op {
                Op::Upsert { uid, ns, object, secs, warn, count } => {
                    let e = build(uid, ns, object, secs, warn, count);
                    buffer.upsert(e.clone());
                    model.upsert(e);
                }
                Op::Delete(uid) => {
                    let uid = format!("e{uid}");
                    let key = EventKey::of(&ev(&uid, "x", "x", 0));
                    buffer.remove(&key);
                    model.delete(&uid);
                }
                Op::Capacity(capacity) => {
                    buffer.set_capacity(capacity);
                    model.capacity(capacity);
                }
            }
            buffer.assert_consistent();
            for filter in filters() {
                let got: Vec<Event> =
                    buffer.query(&filter).map(|e| Event::clone(e)).collect();
                prop_assert_eq!(got, model.query(&filter), "filter {:?}", filter);
            }
            for object in 0..5 {
                let uid = format!("o{object}");
                let all = model.query(&EventFilter::new().object(uid.as_str()));
                let warnings = all.iter().filter(|e| e.is_warning()).count();
                prop_assert_eq!(buffer.object_counts(&uid, None), (all.len(), warnings));
            }
        }
    }
}

#[test]
fn shared_event_arcs_are_not_copied_by_queries() {
    let mut buffer = EventBuffer::new(4);
    buffer.upsert(ev("a", "ns", "p", 1));
    let first: Vec<Arc<Event>> = buffer.query(&EventFilter::new()).cloned().collect();
    let second: Vec<Arc<Event>> = buffer.query(&EventFilter::new()).cloned().collect();
    assert!(Arc::ptr_eq(&first[0], &second[0]));
}
