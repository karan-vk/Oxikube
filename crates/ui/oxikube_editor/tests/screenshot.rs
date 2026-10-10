//! Screenshot of the manifest editor (E10-S04), rendered through the headless renderer:
//!
//! - `manifest_editor_error_dark`: a Deployment whose `replicas` is a word, validated against
//!   the Deployment schema (a fake `SchemaPort`): one error squiggle under `three`, its gutter
//!   marker and its end-of-line message, the toolbar saying "1 error".
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread. Needs a GPU device (Metal, or Vulkan such as Mesa lavapipe on Linux), so it only builds
//! with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_editor --features screenshot --test screenshot`.
//!
//! Regenerate the golden with `OXIKUBE_UPDATE_GOLDENS=1`.

use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use gpui::{App, AppContext as _, Entity, SharedString, px, size};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::schema::JsonSchema;
use oxikube_editor::view::{
    ManifestEditor, ManifestEditorParts, SchemaSource, VALIDATION_DEBOUNCE,
};
use oxikube_ports::SchemaPort;
use oxikube_testkit::fakes::FakeSchemaPort;
use oxikube_testkit::gpui_test::{GoldenCase, ScreenshotApp, run_golden_cases};
use oxikube_testkit::screenshot::RgbaImage;
use oxikube_ui::root::Root;
use oxikube_workspace::CommandDispatcher;
use serde_json::json;

const SIZE: (u32, u32) = (720, 420);

const DEPLOYMENT: &str = "\
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
  labels:
    app: web
spec:
  replicas: three
  selector:
    matchLabels:
      app: web
  template:
    metadata:
      labels:
        app: web
    spec:
      containers:
        - name: web
          image: nginx:1.27
          ports:
            - containerPort: 80
";

/// The part of the Deployment schema the manifest uses (the testkit's OpenAPI fixture keeps only
/// `replicas`, `paused` and `strategy` of the spec).
fn deployment_schema() -> Arc<JsonSchema> {
    let labels = json!({"type": "object", "additionalProperties": {"type": "string"}});
    let metadata = json!({
        "type": "object",
        "properties": {"name": {"type": "string"}, "labels": labels}
    });
    let container = json!({
        "type": "object",
        "required": ["name"],
        "properties": {
            "name": {"type": "string"},
            "image": {"type": "string"},
            "ports": {"type": "array", "items": {
                "type": "object",
                "required": ["containerPort"],
                "properties": {"containerPort": {"type": "integer"}}
            }}
        }
    });
    Arc::new(JsonSchema::from_value(&json!({
        "type": "object",
        "required": ["spec"],
        "properties": {
            "apiVersion": {"type": "string"},
            "kind": {"type": "string"},
            "metadata": metadata,
            "spec": {
                "type": "object",
                "required": ["selector", "template"],
                "properties": {
                    "replicas": {"type": "integer"},
                    "selector": {"type": "object", "properties": {"matchLabels": labels}},
                    "template": {"type": "object", "properties": {
                        "metadata": metadata,
                        "spec": {"type": "object", "properties": {
                            "containers": {"type": "array", "items": container}
                        }}
                    }}
                }
            }
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
        "kind-oxikube".into()
    }
    fn port(&self, _: &App) -> Option<Arc<dyn SchemaPort>> {
        Some(self.port.clone())
    }
}

fn render_error_dark() -> Result<RgbaImage> {
    let mut app = ScreenshotApp::with_assets(Arc::new(oxikube_ui::Assets));
    app.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
    });
    let cluster = ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"));
    let gvk = Gvk::new("apps", "v1", "Deployment");
    let port = Arc::new(FakeSchemaPort::new());
    port.insert(cluster.clone(), gvk, deployment_schema());
    let mut editor: Option<Entity<ManifestEditor>> = None;
    let window = app.open_window(size(px(SIZE.0 as f32), px(SIZE.1 as f32)), |window, cx| {
        let parts = ManifestEditorParts {
            title: "deployment.yaml".into(),
            text: DEPLOYMENT.to_owned(),
            schemas: Some(Rc::new(Schemas { cluster, port })),
            dispatcher: Rc::new(NoBus),
        };
        let entity = cx.new(|cx| ManifestEditor::new(parts, window, cx));
        editor = Some(entity.clone());
        cx.new(|cx| Root::new(entity, window, cx))
    })?;
    // The first validation asks for the schema; the second one, when it arrives, finds the error.
    app.advance_clock(VALIDATION_DEBOUNCE);
    app.run_until_parked();
    let editor = editor.expect("built");
    let errors = app.update(|cx| editor.read(cx).model().problems().errors);
    anyhow::ensure!(errors == 1, "{errors} errors");
    app.capture(window)
}

fn main() -> ExitCode {
    run_golden_cases(
        env!("CARGO_MANIFEST_DIR"),
        &[GoldenCase {
            name: "manifest_editor_error_dark",
            size: SIZE,
            render: render_error_dark,
        }],
    )
}
