//! Row text and bar edge cases.

use oxikube_domain::Quantity;

use super::q;
use crate::allocation::{AllocationKind, AllocationRow, CapacityKind, UnsetCount, Usage};

fn row(capacity: Option<&str>, requests: &str, limits: Option<&str>) -> AllocationRow {
    AllocationRow {
        kind: AllocationKind::Cpu,
        capacity_kind: CapacityKind::Allocatable,
        capacity: capacity.map(q),
        limits_capacity: capacity.map(q),
        requests: q(requests),
        limits: limits.map(q),
        usage: Usage::NotApplicable,
        unset: UnsetCount::default(),
    }
}

#[test]
fn a_zero_capacity_is_a_dash_never_a_division() {
    let r = row(Some("0"), "1", Some("1"));
    assert_eq!(r.requests_percent(), None);
    assert_eq!(r.requests_text(), "1000m of 0");
    let bar = r.bar().unwrap();
    assert_eq!(bar.fill, 1.0, "something asked of nothing is full");
    assert!(bar.full);
    let idle = row(Some("0"), "0", Some("0")).bar().unwrap();
    assert_eq!((idle.fill, idle.full), (0.0, false));
}

#[test]
fn percents_round_down_like_kubectl() {
    let r = row(Some("1000"), "999", Some("1000"));
    assert!(
        r.requests_text().ends_with("(99%)"),
        "{}",
        r.requests_text()
    );
    let tiny = row(Some("1000"), "1m", None);
    assert!(tiny.requests_text().ends_with("(0%)"));
}

#[test]
fn requests_over_capacity_fill_the_bar_and_say_so() {
    let r = row(Some("1"), "2", Some("3"));
    assert!(r.requests_exceed_capacity());
    let bar = r.bar().unwrap();
    assert_eq!(
        (bar.fill, bar.limits_mark, bar.full),
        (1.0, Some(1.0), true)
    );
    assert_eq!(r.requests_text(), "2000m of 1000m (200%)");
}

#[test]
fn unset_text_pluralises() {
    let mut r = row(Some("1"), "0", None);
    r.unset = UnsetCount {
        requests: 1,
        limits: 0,
    };
    assert_eq!(r.unset_text().unwrap(), "1 pod set no request");
    r.unset = UnsetCount {
        requests: 0,
        limits: 3,
    };
    assert_eq!(r.unset_text().unwrap(), "3 pods set no limit");
    r.unset = UnsetCount::default();
    assert_eq!(r.unset_text(), None);
}

#[test]
fn counts_are_whole_numbers_and_memory_is_binary() {
    assert_eq!(AllocationKind::Pods.format(&Quantity::from_value(7)), "7");
    assert_eq!(AllocationKind::Gpu("x".into()).format(&q("2")), "2");
    assert_eq!(AllocationKind::Memory.format(&q("1536Mi")), "1.5 GiB");
    assert_eq!(
        AllocationKind::Gpu("nvidia.com/gpu".into()).title(),
        "GPU nvidia.com/gpu"
    );
    assert!(
        AllocationKind::Cpu.has_live_usage() && !AllocationKind::EphemeralStorage.has_live_usage()
    );
}

#[test]
fn a_quota_never_gets_a_limits_tick_and_words_the_excess_differently() {
    // Equal caps: the tick would appear by coincidence of values if it were inferred from them.
    let mut r = row(Some("2"), "1", Some("3"));
    r.capacity_kind = CapacityKind::Quota;
    assert_eq!(
        r.bar().unwrap().limits_mark,
        None,
        "a quota caps limits on their own scale"
    );
    assert_eq!(
        r.limits_note().unwrap(),
        "limits are above the quota (150%): it was lowered or the pods predate it"
    );
    // The same numbers on a node: over-commitment, normal, and drawn as a tick.
    let node = row(Some("2"), "1", Some("3"));
    assert_eq!(node.bar().unwrap().limits_mark, Some(1.0));
    assert_eq!(
        node.limits_note().unwrap(),
        "limits are over-committed (150%); this is normal"
    );
    assert_eq!(
        row(Some("2"), "1", Some("2")).limits_note(),
        None,
        "at the cap is not over it"
    );
}
