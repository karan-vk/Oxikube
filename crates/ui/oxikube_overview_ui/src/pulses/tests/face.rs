//! What each state says: the wording, the level, the gauge, and that colour is never alone.

use oxikube_app::columns::{Threshold, UsageThresholds};
use oxikube_app::{CountState, FetchedCount, KindCount};

use crate::pulses::{ClusterFigure, Fallback, Level, Resource, cluster_face, kind_face};

fn counted(total: usize, rated: usize, healthy: usize) -> CountState {
    CountState::Counted(KindCount {
        total,
        rated,
        healthy,
        ..Default::default()
    })
}

fn face(plural: &str, state: &CountState) -> crate::pulses::TileFace {
    kind_face(plural, state, &Fallback::None)
}

const GIB: u128 = 1024 * 1024 * 1024;
const CORE: u128 = 1_000_000_000;

fn reading(used: u128, allocatable: u128, percent: Option<f64>) -> ClusterFigure {
    ClusterFigure::Reading {
        used,
        allocatable,
        percent,
        nodes: 3,
        nodes_reporting: 3,
    }
}

#[test]
fn a_kind_with_a_health_rule_shows_the_breakdown_and_a_gauge() {
    let tile = face("pods", &counted(14, 14, 12));
    assert_eq!(tile.value, "14");
    assert_eq!(tile.caption, "12 healthy · 2 unhealthy");
    assert_eq!(tile.level, Level::Warn);
    let gauge = tile.gauge.expect("a gauge");
    assert!((gauge.fraction - 12.0 / 14.0).abs() < 1e-6);
    assert_eq!(gauge.text, "86% healthy");
}

#[test]
fn the_words_follow_the_kind() {
    assert_eq!(
        face("nodes", &counted(3, 3, 2)).caption,
        "2 ready · 1 not ready"
    );
    assert_eq!(
        face("deployments", &counted(4, 4, 3)).caption,
        "3 ready · 1 not ready"
    );
    assert_eq!(face("jobs", &counted(5, 5, 4)).caption, "4 ok · 1 failed");
    assert_eq!(
        face("cronjobs", &counted(2, 2, 1)).caption,
        "1 active · 1 suspended"
    );
}

#[test]
fn the_level_goes_from_good_to_attention_to_critical() {
    assert_eq!(face("pods", &counted(10, 10, 10)).level, Level::Good);
    assert_eq!(face("pods", &counted(10, 10, 9)).level, Level::Warn);
    assert_eq!(face("pods", &counted(10, 10, 5)).level, Level::Critical);
    assert_eq!(face("pods", &counted(10, 10, 0)).level, Level::Critical);
    assert_eq!(face("pods", &counted(0, 0, 0)).level, Level::Neutral);
}

#[test]
fn every_level_that_is_colour_also_has_a_word_and_an_icon() {
    for level in [Level::Good, Level::Warn, Level::Critical] {
        assert!(level.status().is_some(), "{level:?} has a word");
        assert!(level.icon().is_some(), "{level:?} has an icon");
    }
    for level in [Level::Neutral, Level::Muted] {
        assert!(level.status().is_none() && level.icon().is_none());
    }
}

#[test]
fn a_kind_without_a_health_rule_is_counted_and_not_judged() {
    let tile = face("persistentvolumes", &counted(7, 0, 0));
    assert_eq!(tile.value, "7");
    assert_eq!(tile.caption, "");
    assert_eq!(tile.level, Level::Neutral);
    assert_eq!(tile.gauge, None);
}

#[test]
fn states_without_a_number_say_why_and_never_read_zero() {
    let loading = face("pods", &CountState::Loading);
    assert_eq!(
        (loading.value.as_str(), loading.caption.as_str()),
        ("…", "Loading")
    );
    let denied = face(
        "pods",
        &CountState::NoAccess {
            message: "pods is forbidden".into(),
        },
    );
    assert_eq!(denied.value, "no access");
    assert_eq!(denied.hover.as_deref(), Some("pods is forbidden"));
    for state in [
        CountState::OverBudget {
            message: "too many feeds".into(),
        },
        CountState::Failed {
            message: "boom".into(),
        },
        CountState::NotWatched,
    ] {
        let tile = face("pods", &state);
        assert_eq!(tile.value, "–", "{state:?}");
        assert_ne!(tile.value, "0");
    }
}

#[test]
fn a_kind_with_no_feed_shows_the_one_off_count_marked_as_such() {
    let over = CountState::OverBudget {
        message: "too many feeds".into(),
    };
    let exact = kind_face(
        "pods",
        &over,
        &Fallback::Counted(FetchedCount {
            total: 10_000,
            exact: true,
        }),
    );
    assert_eq!(exact.value, "10000");
    assert!(
        exact.caption.starts_with("Counted once"),
        "{}",
        exact.caption
    );
    assert_eq!(exact.hover.as_deref(), Some("too many feeds"));
    assert_eq!(exact.level, Level::Muted);
    assert_eq!(exact.gauge, None, "no breakdown from a count alone");

    let bound = kind_face(
        "pods",
        &over,
        &Fallback::Counted(FetchedCount {
            total: 2,
            exact: false,
        }),
    );
    assert_eq!(bound.value, "≥2");

    let failed = kind_face("pods", &over, &Fallback::Failed("nope".into()));
    assert_eq!(failed.value, "–");
    assert_eq!(failed.hover.as_deref(), Some("nope"));
    assert_eq!(kind_face("pods", &over, &Fallback::Loading).value, "…");
}

#[test]
fn the_cluster_gauges_show_the_percent_the_amounts_and_the_threshold_level() {
    let thresholds = UsageThresholds::default();
    let cpu = cluster_face(
        Resource::Cpu,
        &reading(3 * CORE / 2, 8 * CORE, Some(18.75)),
        &thresholds,
    );
    assert_eq!(cpu.value, "19%");
    assert_eq!(cpu.caption, "1.5 / 8 cores");
    assert_eq!(cpu.level, Level::Good);
    assert_eq!(
        cpu.gauge.as_ref().map(|g| g.text.as_str()),
        Some("19% of CPU")
    );

    let memory = cluster_face(
        Resource::Memory,
        &reading(12 * GIB, 16 * GIB, Some(75.0)),
        &thresholds,
    );
    assert_eq!(memory.value, "75%");
    assert_eq!(memory.caption, "12 GiB / 16 GiB");
    assert_eq!(memory.level, Level::Warn, "75 is past warn 70");

    let hot = cluster_face(
        Resource::Memory,
        &reading(15 * GIB, 16 * GIB, Some(93.75)),
        &thresholds,
    );
    assert_eq!(hot.level, Level::Critical, "94 is past critical 90");
    assert_eq!(hot.value, "94%");
}

#[test]
fn the_gauges_follow_the_thresholds_of_the_cluster() {
    let relaxed = UsageThresholds {
        cpu: Threshold::new(95, 99),
        memory: Threshold::new(50, 60),
    };
    let figure = reading(8 * CORE, 10 * CORE, Some(80.0));
    assert_eq!(
        cluster_face(Resource::Cpu, &figure, &relaxed).level,
        Level::Good
    );
    assert_eq!(
        cluster_face(Resource::Memory, &figure, &relaxed).level,
        Level::Critical
    );
}

#[test]
fn a_small_percent_keeps_a_decimal_so_it_is_not_zero() {
    let tile = cluster_face(
        Resource::Cpu,
        &reading(CORE / 20, 8 * CORE, Some(0.625)),
        &UsageThresholds::default(),
    );
    assert_eq!(tile.value, "0.6%");
}

#[test]
fn partial_reporting_is_said() {
    let tile = cluster_face(
        Resource::Cpu,
        &ClusterFigure::Reading {
            used: CORE,
            allocatable: 4 * CORE,
            percent: Some(25.0),
            nodes: 10,
            nodes_reporting: 7,
        },
        &UsageThresholds::default(),
    );
    assert_eq!(tile.caption, "1 / 4 cores · 7 of 10 nodes");
}

#[test]
fn no_metrics_is_a_visible_state_never_a_zero() {
    let thresholds = UsageThresholds::default();
    let gone = cluster_face(
        Resource::Cpu,
        &ClusterFigure::Unavailable(
            "metrics unavailable: metrics-server is not installed (no metrics.k8s.io API)".into(),
        ),
        &thresholds,
    );
    assert_eq!(gone.value, "–");
    assert!(
        gone.caption.starts_with("metrics unavailable: "),
        "{}",
        gone.caption
    );
    assert_eq!(gone.hover.as_deref(), Some(gone.caption.as_str()));
    assert_eq!(gone.gauge, None);

    let waiting = cluster_face(Resource::Cpu, &ClusterFigure::Waiting, &thresholds);
    assert_eq!(waiting.caption, "Waiting for metrics");

    let nothing = cluster_face(Resource::Cpu, &reading(0, 0, None), &thresholds);
    assert_eq!(nothing.value, "–", "no allocatable is a dash, not 0%");
}
