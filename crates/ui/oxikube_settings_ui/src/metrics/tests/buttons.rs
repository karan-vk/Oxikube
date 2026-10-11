//! The buttons send bus commands and nothing else.

use gpui::TestAppContext;
use oxikube_domain::command::Command;

use super::Harness;
use crate::metrics::test_support::{cluster, detecting, disabled, found};

#[gpui::test]
fn look_again_sends_cluster_redetect_metrics(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, found());
    h.click("metrics-redetect");
    assert_eq!(
        h.dispatcher.sent(),
        vec![Command::ClusterRedetectMetrics { cluster: cluster() }]
    );
}

#[gpui::test]
fn look_again_waits_while_a_run_is_in_progress(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, detecting());
    h.click("metrics-redetect");
    assert!(
        h.dispatcher.sent().is_empty(),
        "a disabled button sends nothing"
    );
}

#[gpui::test]
fn look_again_is_off_when_the_cluster_is_not_connected(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, found());
    h.backend.set_connected(false);
    h.set(found());
    h.click("metrics-redetect");
    assert!(h.dispatcher.sent().is_empty());
}

#[gpui::test]
fn the_switch_turns_history_metrics_off_and_back_on(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, found());
    h.click("metrics-toggle");
    h.set(disabled());
    h.click("metrics-toggle");
    assert_eq!(
        h.dispatcher.sent(),
        vec![
            Command::ClusterToggleMetrics {
                cluster: cluster(),
                enabled: Some(false)
            },
            Command::ClusterToggleMetrics {
                cluster: cluster(),
                enabled: Some(true)
            },
        ]
    );
}
