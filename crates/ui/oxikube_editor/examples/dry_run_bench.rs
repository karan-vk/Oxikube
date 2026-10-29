//! Micro benchmark (E19-S07): the guard's dry run of a 1 MB object in the confirmation dialog.
//!
//! `cargo run -p oxikube_editor --profile release-fast --example dry_run_bench`
//!
//! Runs on GPUI's test platform (no GPU, test text system), like `live_bench`: it reports the
//! diff of a 1 MB ConfigMap against its dry-run result (three keys changed: the first, one in the
//! middle, the last), which the dialog runs on the background executor, and the frames of the
//! open dialog, idle and scrolling through the diff (docs/PERFORMANCE.md: p95 frame <= 8 ms).
//! `tests/dry_run.rs` checks that the diff never runs on the UI thread; this measures the cost.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a benchmark reports on stdout and stderr"
)]

use std::sync::Arc;
use std::time::Instant;

use gpui::{
    AppContext as _, Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext,
    point, px, size,
};
use oxikube_app::PreviewChange;
use oxikube_app::apply::Change;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_editor::diff::ManifestDiff;
use oxikube_ui::root::Root;
use oxikube_workspace::{ConfirmDialog, ConfirmPrompt};
use serde_json::{Value, json};

const MB: usize = 1 << 20;
const FRAMES: usize = 240;

fn config_map(version: &str) -> Value {
    let mut data = serde_json::Map::new();
    let (mut i, mut bytes) = (0, 0);
    while bytes < MB {
        let value = format!("value-{i:08}-{}", "x".repeat(48));
        bytes += value.len() + 16;
        data.insert(format!("key-{i:08}"), json!(value));
        i += 1;
    }
    json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {"name": "web", "namespace": "shop", "resourceVersion": version},
        "data": data
    })
}

fn objects() -> (Value, Value) {
    let (live, mut result) = (config_map("7"), config_map("8"));
    let keys: Vec<String> = live["data"]
        .as_object()
        .expect("a data map")
        .keys()
        .cloned()
        .collect();
    for key in [&keys[1], &keys[keys.len() / 2], &keys[keys.len() - 1]] {
        result["data"][key] = json!("changed");
    }
    (live, result)
}

fn report(name: &str, mut ms: Vec<f64>) {
    ms.sort_by(|a, b| a.total_cmp(b));
    let mean = ms.iter().sum::<f64>() / ms.len() as f64;
    let p95 = ms[(ms.len() * 95 / 100).min(ms.len() - 1)];
    let max = ms[ms.len() - 1];
    println!(
        "dry_run_bench {name}: {} samples, ms mean {mean:.3} p95 {p95:.3} max {max:.3}",
        ms.len()
    );
}

fn frame(vcx: &mut VisualTestContext) -> f64 {
    let started = Instant::now();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    started.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let (live, result) = objects();
    let bytes = serde_json::to_vec(&live).expect("serialises").len();
    let mut ms = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let diff = ManifestDiff::of_manifests(Some(&live), &result);
        ms.push(started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(diff.hunks(), 3, "the three changed keys");
    }
    println!("dry_run_bench object: {} KiB", bytes / 1024);
    report("diff (background)", ms);

    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
    });
    let target = ResourceRef::namespaced(
        ClusterId::new("/kubeconfig", &ContextName::new("bench")),
        Gvk::new("", "v1", "ConfigMap"),
        "shop",
        "web",
    );
    let change = PreviewChange {
        target,
        change: Change::Update,
        live: Some(Arc::new(live)),
        result: Arc::new(result),
    };
    let prompt = ConfirmPrompt::new("Apply?", "Apply ConfigMap shop/web on bench")
        .with_changes(vec![change], 0);
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| ConfirmDialog::new(prompt, |_, _, _| {}, |_| {}, window, cx));
        Root::new(view, window, cx)
    });
    let mut vcx = VisualTestContext::from_window(window.into(), &cx);
    vcx.simulate_resize(size(px(900.0), px(720.0)));
    report("dialog frame (diff pending)", vec![frame(&mut vcx)]);
    vcx.run_until_parked();
    let idle: Vec<f64> = (0..FRAMES).map(|_| frame(&mut vcx)).collect();
    report("dialog frame (diff shown, idle)", idle);
    let bounds = vcx.debug_bounds("dry-run-diff").expect("the diff is drawn");
    let over_rows = bounds.center();
    let scrolling: Vec<f64> = (0..FRAMES)
        .map(|i| {
            let dy = if i < FRAMES / 2 { -40.0 } else { 40.0 };
            vcx.simulate_event(ScrollWheelEvent {
                position: point(over_rows.x, over_rows.y + px(20.0)),
                delta: ScrollDelta::Pixels(point(px(0.0), px(dy))),
                modifiers: Modifiers::default(),
                ..ScrollWheelEvent::default()
            });
            frame(&mut vcx)
        })
        .collect();
    report("dialog frame (scrolling the diff)", scrolling);
}
