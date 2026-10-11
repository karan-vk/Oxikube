//! A settings edit reaches the aggregator the app uses, live.

use gpui::{BorrowAppContext as _, TestAppContext};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::utilisation::DEFAULT_GPU_VENDORS;
use oxikube_settings::SettingsStore;
use serde_json::json;

use super::follow_gpu_vendors;
use crate::app_state::AppState;

const LAB: &str = "0011223344556677";

fn edit_settings_file(cx: &mut TestAppContext, text: &str) {
    let text = text.to_owned();
    cx.update(|cx| {
        cx.update_global::<SettingsStore, _>(|store, _| store.set_user_settings(&text).unwrap());
    });
    cx.run_until_parked();
}

fn pod(requests: serde_json::Value) -> oxikube_domain::Resource {
    oxikube_domain::Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "p", "namespace": "ns"},
        "spec": {"nodeName": "n", "containers": [{"name": "c", "resources": {"requests": requests}}]},
        "status": {"phase": "Running"},
    }))
    .unwrap()
}

#[gpui::test]
fn the_aggregator_follows_metrics_gpu_vendors_live(cx: &mut TestAppContext) {
    let aggregator = cx.update(|cx| {
        let state = AppState::test(cx);
        let aggregator = state.services().utilisation.clone();
        follow_gpu_vendors(cx, aggregator.vendors().clone()).detach();
        aggregator
    });
    let lab: ClusterId = LAB.parse().unwrap();
    let pods = [pod(json!({"nvidia.com/gpu": "1", "example.com/fpga": "3"}))];
    let gpu_total = |cluster: &ClusterId| {
        aggregator.namespaces(cluster, &pods)[0]
            .requests
            .gpu_total()
    };

    // The shipped default.json list.
    assert_eq!(
        aggregator.vendors().table().default_vendors().names().len(),
        DEFAULT_GPU_VENDORS.len()
    );
    assert_eq!(gpu_total(&lab), 1, "nvidia.com/gpu only");

    // The user adds the FPGA; the very next computation counts it.
    edit_settings_file(
        cx,
        r#"{ "metrics": { "gpu_vendors": ["nvidia.com/gpu", "example.com/fpga"] } }"#,
    );
    assert_eq!(gpu_total(&lab), 4);

    // A cluster turns GPU accounting off for itself.
    edit_settings_file(
        cx,
        &format!(
            r#"{{ "metrics": {{ "gpu_vendors": ["nvidia.com/gpu", "example.com/fpga"] }},
                "clusters": {{ "{LAB}": {{ "metrics": {{ "gpu_vendors": [] }} }} }} }}"#
        ),
    );
    assert_eq!(gpu_total(&lab), 0);
    let other: ClusterId = "3f2a9c1b7d4e8a60".parse().unwrap();
    assert_eq!(gpu_total(&other), 4, "other clusters keep the global list");
}
