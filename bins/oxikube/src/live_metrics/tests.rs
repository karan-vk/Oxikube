//! The `metrics` setting reaches the live metrics service, live; the window's shown tab is what
//! the service may poll.

use gpui::{BorrowAppContext as _, TestAppContext};
use oxikube_app::columns::{Threshold, UsageThresholds};
use oxikube_domain::ids::ClusterId;
use oxikube_settings::SettingsStore;
use std::time::Duration;

use super::{config_table, follow_settings};
use crate::app_state::AppState;

const LAB: &str = "0011223344556677";

fn edit_settings_file(cx: &mut TestAppContext, text: &str) {
    let text = text.to_owned();
    cx.update(|cx| {
        cx.update_global::<SettingsStore, _>(|store, _| store.set_user_settings(&text).unwrap());
    });
    cx.run_until_parked();
}

#[gpui::test]
fn the_shipped_defaults_are_fifteen_seconds_and_k9s_thresholds(cx: &mut TestAppContext) {
    let table = cx.update(|cx| {
        AppState::test(cx);
        config_table(cx)
    });
    let lab: ClusterId = LAB.parse().unwrap();
    assert_eq!(table.get(&lab).poll_interval, Duration::from_secs(15));
    assert_eq!(table.get(&lab).thresholds, UsageThresholds::default());
}

#[gpui::test]
fn the_service_follows_the_metrics_setting_live(cx: &mut TestAppContext) {
    let service = cx.update(|cx| {
        let state = AppState::test(cx);
        let service = state.services().live_metrics.clone();
        follow_settings(cx, service.clone()).detach();
        service
    });
    let lab: ClusterId = LAB.parse().unwrap();
    let other: ClusterId = "3f2a9c1b7d4e8a60".parse().unwrap();
    let usage = service.usage(&lab);
    assert_eq!(usage.thresholds(), UsageThresholds::default());

    edit_settings_file(
        cx,
        &format!(
            r#"{{ "metrics": {{ "thresholds": {{ "cpu": {{ "warn": 50, "critical": 75 }} }} }},
                "clusters": {{ "{LAB}": {{ "metrics": {{ "thresholds": {{ "memory": {{ "warn": 60 }} }} }} }} }} }}"#
        ),
    );
    assert_eq!(
        usage.thresholds(),
        UsageThresholds {
            cpu: Threshold::new(50, 75),
            memory: Threshold::new(60, 90),
        },
        "the cluster's own level on top of the global ones"
    );
    assert_eq!(
        service.usage(&other).thresholds().memory,
        Threshold::default()
    );
}
