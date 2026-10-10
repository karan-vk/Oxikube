//! Micro benchmark (E10-S04): typing in a 2 000-line manifest in the manifest editor, and what
//! the debounced validation costs off the UI thread.
//!
//! `cargo run -p oxikube_editor --profile release-fast --example typing_bench`
//!
//! Runs on GPUI's test platform (no GPU, test text system), so it measures the UI-thread work a
//! keystroke causes: the edit, the tree-sitter reparse, the debounce restart and the frame that
//! shows it (layout, highlighting, the diagnostics overlay over the visible lines; 53 documents
//! with one schema error each, so the overlay has work). It is a regression check, not the ADR
//! 0013 frame budget, which needs `oxikube --perf` in a real window. The validation itself runs
//! on the background executor; its time is reported separately.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui::{App, AppContext as _, SharedString, TestAppContext, VisualTestContext};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::schema::JsonSchema;
use oxikube_editor::view::{
    KnownSchemas, ManifestEditor, ManifestEditorParts, SchemaSource, VALIDATION_DEBOUNCE,
    validate_text,
};
use oxikube_ports::SchemaPort;
use oxikube_testkit::fakes::FakeSchemaPort;
use oxikube_ui::root::Root;
use oxikube_workspace::CommandDispatcher;
use serde_json::json;

const DOCUMENTS: usize = 53;
const KEYSTROKES: usize = 300;

fn document(i: usize) -> String {
    format!(
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web-{i}\n  labels:\n    app: web-{i}\n    tier: frontend\nspec:\n  replicas: {replicas}\n  selector:\n    matchLabels:\n      app: web-{i}\n  template:\n    metadata:\n      labels:\n        app: web-{i}\n    spec:\n      containers:\n        - name: web\n          image: nginx:1.27\n          ports:\n            - containerPort: 80\n          env:\n            - name: MODE\n              value: production\n            - name: LEVEL\n              value: info\n          resources:\n            limits:\n              cpu: 500m\n              memory: 256Mi\n            requests:\n              cpu: 100m\n              memory: 128Mi\n        - name: sidecar\n          image: busybox:1.36\n          args: [sleep, infinity]\n---\n",
        replicas = if i.is_multiple_of(2) {
            "three".to_owned()
        } else {
            i.to_string()
        },
    )
}

fn manifest() -> String {
    (0..DOCUMENTS).map(document).collect()
}

fn deployment_schema() -> Arc<JsonSchema> {
    let free = json!({"type": "object", "x-kubernetes-preserve-unknown-fields": true});
    Arc::new(JsonSchema::from_value(&json!({
        "type": "object",
        "properties": {
            "apiVersion": {"type": "string"},
            "kind": {"type": "string"},
            "metadata": free,
            "spec": {"type": "object", "properties": {
                "replicas": {"type": "integer"},
                "selector": free,
                "template": free
            }}
        }
    })))
}

struct NoBus;

impl CommandDispatcher for NoBus {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

struct Schemas {
    cluster: ClusterId,
    port: Arc<FakeSchemaPort>,
}

impl SchemaSource for Schemas {
    fn cluster(&self) -> &ClusterId {
        &self.cluster
    }
    fn label(&self, _: &App) -> SharedString {
        "bench".into()
    }
    fn port(&self, _: &App) -> Option<Arc<dyn SchemaPort>> {
        Some(self.port.clone())
    }
}

fn report(name: &str, mut ms: Vec<f64>) {
    ms.sort_by(|a, b| a.total_cmp(b));
    let mean = ms.iter().sum::<f64>() / ms.len() as f64;
    let p95 = ms[(ms.len() * 95 / 100).min(ms.len() - 1)];
    let max = ms[ms.len() - 1];
    println!(
        "typing_bench {name}: {} samples, ms mean {mean:.3} p95 {p95:.3} max {max:.3}",
        ms.len()
    );
}

fn main() {
    let text = manifest();
    println!(
        "typing_bench manifest: {} lines, {} bytes",
        text.lines().count(),
        text.len()
    );
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
    });
    let cluster = ClusterId::new("/kubeconfig", &ContextName::new("bench"));
    let gvk = Gvk::new("apps", "v1", "Deployment");
    let port = Arc::new(FakeSchemaPort::new());
    port.insert(cluster.clone(), gvk.clone(), deployment_schema());
    let mut editor = None;
    let window = cx.add_window(|window, cx| {
        let parts = ManifestEditorParts {
            title: "bench.yaml".into(),
            text: text.clone(),
            schemas: Some(Rc::new(Schemas { cluster, port })),
            dispatcher: Rc::new(NoBus),
        };
        let entity = cx.new(|cx| ManifestEditor::new(parts, window, cx));
        editor = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let editor = editor.expect("built");
    let mut vcx = VisualTestContext::from_window(window.into(), &cx);
    vcx.update(|window, cx| {
        window.activate_window();
        let buffer = gpui::Focusable::focus_handle(editor.read(cx), cx);
        window.focus(&buffer, cx);
    });
    vcx.executor().advance_clock(VALIDATION_DEBOUNCE);
    vcx.run_until_parked();
    let problems = vcx.update(|_, cx| editor.read(cx).model().problems());
    println!(
        "typing_bench diagnostics: {} errors, {} warnings",
        problems.errors, problems.warnings
    );
    vcx.update(|window, cx| window.draw(cx).clear(cx));

    // Reference: a redraw with nothing changed.
    let mut ms = Vec::with_capacity(KEYSTROKES);
    for _ in 0..KEYSTROKES {
        let started = Instant::now();
        vcx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("idle-redraw", ms);

    // One keystroke and the frame that shows it (the debounce timer restarts; no validation runs
    // while typing continues).
    let mut ms = Vec::with_capacity(KEYSTROKES);
    for i in 0..KEYSTROKES {
        let key = if i % 10 == 9 { "\n" } else { "x" };
        let started = Instant::now();
        vcx.simulate_input(key);
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("keystroke-to-frame", ms);

    // The validation a pause triggers, as the background executor runs it.
    let mut known = KnownSchemas::new();
    known.insert(gvk, Some(deployment_schema()));
    let text: Arc<str> = Arc::from(text);
    let mut ms = Vec::with_capacity(20);
    for _ in 0..20 {
        let started = Instant::now();
        let result = validate_text(1, text.clone(), &known);
        std::hint::black_box(result);
        ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("validate-2k-lines (background)", ms);
}
