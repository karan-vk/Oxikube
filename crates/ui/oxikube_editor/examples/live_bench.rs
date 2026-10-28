//! Micro benchmark (E10-S07): typing in a Deployment tab whose live object changes every second,
//! with the diff vs live open, and what the diff and the merge cost off the UI thread.
//!
//! `cargo run -p oxikube_editor --profile release-fast --example live_bench`
//!
//! Runs on GPUI's test platform (no GPU, test text system), like `typing_bench`: it measures the
//! UI-thread work of a keystroke and of a watch event (one per ten keystrokes: typing at ten keys
//! a second against an object updated every second), each with the frame that shows it. The
//! object's rendering, the normalising and the diff run on the background executor; their time
//! is reported separately for a large object (a Deployment with 400 env entries).

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a benchmark reports on stdout and stderr"
)]

use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui::{App, AppContext as _, TestAppContext, VisualTestContext};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_editor::diff::{ManifestDiff, ObjectVersion, ThreeWayMerge, default_renderer};
use oxikube_editor::view::{DIFF_DEBOUNCE, ManifestEditor, ManifestEditorParts};
use oxikube_testkit::fakes::FakeResourcePort;
use oxikube_testkit::{ScriptedFeed, TICK};
use oxikube_ui::root::Root;
use oxikube_workspace::CommandDispatcher;
use serde_json::json;

#[path = "../tests/support/mod.rs"]
mod support;

use support::FakeLive;

const KEYSTROKES: usize = 300;
const ENV: usize = 400;

fn deployment(generation: u32) -> Resource {
    let env: Vec<_> = (0..ENV)
        .map(|i| json!({"name": format!("VAR_{i}"), "value": format!("value-{i}")}))
        .collect();
    Resource::from_json(json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {"name": "web", "namespace": "shop",
                     "resourceVersion": generation.to_string(), "generation": generation},
        "spec": {
            "replicas": 3,
            "selector": {"matchLabels": {"app": "web"}},
            "template": {"metadata": {"labels": {"app": "web"}},
                         "spec": {"containers": [{"name": "web", "image": "nginx:1.27", "env": env}]}}
        },
        "status": {"observedGeneration": generation, "replicas": 3,
                   "readyReplicas": generation % 4, "updatedReplicas": 3}
    }))
    .expect("a deployment")
}

struct NoBus;

impl CommandDispatcher for NoBus {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

fn report(name: &str, mut ms: Vec<f64>) {
    ms.sort_by(|a, b| a.total_cmp(b));
    let mean = ms.iter().sum::<f64>() / ms.len() as f64;
    let p95 = ms[(ms.len() * 95 / 100).min(ms.len() - 1)];
    let max = ms[ms.len() - 1];
    println!(
        "live_bench {name}: {} samples, ms mean {mean:.3} p95 {p95:.3} max {max:.3}",
        ms.len()
    );
}

fn background_costs() {
    let render = default_renderer();
    let mut ms = Vec::new();
    for g in 0..20 {
        let started = Instant::now();
        std::hint::black_box(ObjectVersion::new(deployment(g), &render));
        ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("render+normalise (background)", ms);
    let base = ObjectVersion::new(deployment(1), &render);
    let live = deployment(2).to_value();
    let mine_text = base.text.replace("value-7\n", "value-7-edited\n");
    let mine = oxikube_editor::diff::document_for(&mine_text, None).expect("parses");
    let mut ms = Vec::new();
    for _ in 0..20 {
        let started = Instant::now();
        std::hint::black_box(ManifestDiff::of_manifests(Some(&live), &mine));
        ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("diff (background)", ms);
    let server = ObjectVersion::new(deployment(2), &render);
    let mut ms = Vec::new();
    for _ in 0..20 {
        let started = Instant::now();
        std::hint::black_box(ThreeWayMerge::new(&base.text, &mine_text, &server.text));
        ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("merge (background)", ms);
}

fn main() {
    let ticks = (KEYSTROKES / 10 + 2) as u32;
    let port = Arc::new(FakeResourcePort::new());
    (1..=ticks)
        .fold(ScriptedFeed::new().initial([deployment(1)]), |feed, t| {
            feed.modify(t, deployment(t + 1))
        })
        .install(&port);
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
    });
    let mut editor = None;
    let window = cx.add_window(|window, cx| {
        let parts = ManifestEditorParts {
            title: "web".into(),
            text: String::new(),
            schemas: None,
            dispatcher: Rc::new(NoBus),
            resource: None,
            live: Some(Rc::new(FakeLive::new(
                ClusterId::new("/kubeconfig", &ContextName::new("bench")),
                port.clone(),
            ))),
        };
        let entity = cx.new(|cx| ManifestEditor::new(parts, window, cx));
        entity.update(cx, |e, cx| {
            e.follow_object(deployment(1), None, window, cx);
            e.toggle_diff(window, cx);
        });
        editor = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let editor = editor.expect("built");
    let mut vcx = VisualTestContext::from_window(window.into(), &cx);
    vcx.update(|window, cx| {
        window.activate_window();
        window.focus(&gpui::Focusable::focus_handle(editor.read(cx), cx), cx);
    });
    vcx.run_until_parked();
    let lines = vcx.update(|_, cx| {
        editor
            .read(cx)
            .editor()
            .read(cx)
            .snapshot(cx)
            .to_text()
            .lines()
            .count()
    });
    println!("live_bench buffer: {lines} lines, diff panel open");
    vcx.update(|window, cx| window.draw(cx).clear(cx));

    let (mut keys, mut events) = (Vec::new(), Vec::new());
    for i in 0..KEYSTROKES {
        let started = Instant::now();
        vcx.simulate_input("x");
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        keys.push(started.elapsed().as_secs_f64() * 1000.0);
        if i % 10 == 9 {
            // A second passes: the object changes on the server, the debounced diff runs.
            port.clock().advance(TICK);
            vcx.executor().advance_clock(DIFF_DEBOUNCE);
            let started = Instant::now();
            vcx.run_until_parked();
            vcx.update(|window, cx| window.draw(cx).clear(cx));
            events.push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    report("keystroke-to-frame (live object, diff open)", keys);
    report(
        "watch-event-to-frame (incl. background on test executor)",
        events,
    );
    let state = vcx.update(|_, cx| editor.read(cx).live_sync().state());
    println!("live_bench state after typing: {state:?}");
    background_costs();
}
