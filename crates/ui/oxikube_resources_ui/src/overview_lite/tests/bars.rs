//! A tile's bar as data: which buckets, in what order, how wide, and in which colour.

use oxikube_app::{BucketCounts, KindCount};
use oxikube_domain::Resource;
use oxikube_domain::view::{Bucket, bucket_of, health_of};
use oxikube_testkit::{daemonset, pod};
use oxikube_ui::Tokens;

use crate::overview_lite::{colour, segments};

/// The count the store would tally for `objects`.
fn tally(objects: &[Resource]) -> KindCount {
    let rated: Vec<_> = objects
        .iter()
        .filter_map(|o| Some((o, health_of(o)?)))
        .collect();
    KindCount {
        total: objects.len(),
        rated: rated.len(),
        healthy: rated.iter().filter(|(_, h)| h.is_healthy()).count(),
        buckets: BucketCounts::of(rated.iter().map(|(o, h)| bucket_of(o, *h))),
    }
}

#[test]
fn pod_segments_follow_the_bars_order_and_skip_empty_buckets() {
    let pods = [
        pod().name("a").running().build(),
        pod().name("b").running().build(),
        pod().name("c").pending().build(),
        pod().name("d").failed().build(),
        pod().name("e").succeeded().build(),
    ];
    let segs = segments("", "Pod", &tally(&pods));
    let seen: Vec<_> = segs.iter().map(|s| (s.bucket, s.count)).collect();
    assert_eq!(
        seen,
        [
            (Bucket::Running, 2),
            (Bucket::Pending, 1),
            (Bucket::Succeeded, 1),
            (Bucket::Failed, 1),
        ],
        "Running, Not ready, Pending, Succeeded, Failed, Unknown; empty ones left out"
    );
    let sum: f32 = segs.iter().map(|s| s.fraction).sum();
    assert!(
        (sum - 1.0).abs() < 1e-5,
        "the fractions fill the bar: {sum}"
    );
    assert!((segs[0].fraction - 0.4).abs() < 1e-5);
}

#[test]
fn a_controller_has_ready_and_not_ready_segments() {
    let set = [
        daemonset().name("ok").replicas(2).ready(2).build(),
        daemonset().name("short").replicas(3).ready(1).build(),
        daemonset().name("also").replicas(3).ready(0).build(),
    ];
    let segs = segments("apps", "DaemonSet", &tally(&set));
    let seen: Vec<_> = segs.iter().map(|s| (s.bucket, s.count)).collect();
    assert_eq!(seen, [(Bucket::Ready, 1), (Bucket::NotReady, 2)]);
}

#[test]
fn nothing_rated_or_no_rule_means_no_bar() {
    assert!(segments("apps", "Deployment", &KindCount::default()).is_empty());
    let configmaps = KindCount {
        total: 4,
        ..KindCount::default()
    };
    assert!(segments("", "ConfigMap", &configmaps).is_empty());
}

#[test]
fn colours_come_from_the_theme_status_tokens() {
    let colors = Tokens::dark().colors;
    assert_eq!(colour(Bucket::Running, &colors), colors.success);
    assert_eq!(colour(Bucket::Ready, &colors), colors.success);
    assert_eq!(colour(Bucket::Pending, &colors), colors.warning);
    assert_eq!(colour(Bucket::NotReady, &colors), colors.warning);
    assert_eq!(colour(Bucket::Failed, &colors), colors.error);
    assert_eq!(colour(Bucket::Succeeded, &colors), colors.info);
    assert_ne!(colour(Bucket::Suspended, &colors), colors.success);
}
