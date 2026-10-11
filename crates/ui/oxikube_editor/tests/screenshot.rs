//! Screenshot of the manifest editor (E10-S04), rendered through the headless renderer:
//!
//! - `manifest_editor_error_dark`: a Deployment whose `replicas` is a word, validated against
//!   the Deployment schema (a fake `SchemaPort`): one error squiggle under `three`, its gutter
//!   marker and its end-of-line message, the toolbar saying "1 error".
//! - `manifest_editor_json_dark` (E10-S10): a commented Deployment converted to JSON: the JSON
//!   buffer, the language chip and the "To YAML" button, and the notice bar warning that two
//!   comments were dropped, with its Undo.
//!
//! - `manifest_editor_open_deployment_dark` and `manifest_editor_open_secret_dark` (E10-S06): an
//!   editor opened from a live object through the fake reader. The Deployment is read-only
//!   ("View YAML in Editor"), kubectl's key order, `managedFields` hidden, `status` dimmed; the
//!   Secret is masked, the placeholders underlined as errors.
//!
//! - `manifest_editor_completion_dark` and `manifest_editor_hover_dark` (E10-S05): `ima` typed in
//!   a container with the completion menu open beside the docs of the selected field; and the
//!   docs popup of `containerPort` at the cursor.
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
use gpui::{AnyWindowHandle, App, AppContext as _, Entity, SharedString, px, size};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::schema::JsonSchema;
use oxikube_editor::json::Language;
use oxikube_editor::open::{OpenMode, ResourceOpen};
use oxikube_editor::view::{
    ManifestEditor, ManifestEditorParts, SchemaSource, VALIDATION_DEBOUNCE,
};
use oxikube_ports::{ResourceReader, SchemaPort};
use oxikube_testkit::fakes::{FakeResourcePort, FakeSchemaPort};
use oxikube_testkit::gpui_test::{GoldenCase, ScreenshotApp, run_golden_cases};
use oxikube_testkit::screenshot::RgbaImage;
use oxikube_ui::editor::EditorApi as _;
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
            "image": {"type": "string", "description": "Container image name."},
            "imagePullPolicy": {
                "type": "string",
                "description": "Image pull policy. One of Always, Never, IfNotPresent.",
                "enum": ["Always", "IfNotPresent", "Never"]
            },
            "ports": {"type": "array", "items": {
                "type": "object",
                "required": ["containerPort"],
                "properties": {"containerPort": {
                    "type": "integer",
                    "format": "int32",
                    "description": "Number of port to expose on the pod's IP address."
                }}
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
    let (mut app, window, editor) = open(DEPLOYMENT)?;
    // The first validation asks for the schema; the second one, when it arrives, finds the error.
    app.advance_clock(VALIDATION_DEBOUNCE);
    app.run_until_parked();
    let errors = app.update(|cx| editor.read(cx).model().problems().errors);
    anyhow::ensure!(errors == 1, "{errors} errors");
    app.capture(window)
}

/// A container with a half-typed key, the cursor at its end, the completion menu opened.
fn render_completion_dark() -> Result<RgbaImage> {
    let text = "apiVersion: apps/v1\nkind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n        - name: web\n          ima\n";
    let (mut app, window, editor) = open(text)?;
    app.advance_clock(VALIDATION_DEBOUNCE);
    app.run_until_parked();
    let at = text.len() - 1;
    place_cursor_and(&mut app, window, &editor, at, |editor, window, cx| {
        editor.show_completions(window, cx);
    })?;
    app.capture(window)
}

/// The cursor on `containerPort`, its docs shown.
fn render_hover_dark() -> Result<RgbaImage> {
    let (mut app, window, editor) = open(DEPLOYMENT)?;
    app.advance_clock(VALIDATION_DEBOUNCE);
    app.run_until_parked();
    let at = DEPLOYMENT.find("containerPort").expect("field") + 4;
    place_cursor_and(&mut app, window, &editor, at, |editor, window, cx| {
        editor.show_hover(window, cx);
    })?;
    app.capture(window)
}

/// Puts the cursor at `at` in `editor`'s buffer, runs `then`, and lets the answer arrive.
fn place_cursor_and(
    app: &mut ScreenshotApp,
    window: AnyWindowHandle,
    editor: &Entity<ManifestEditor>,
    at: usize,
    then: impl FnOnce(&mut ManifestEditor, &mut gpui::Window, &mut gpui::Context<ManifestEditor>),
) -> Result<()> {
    app.update(|cx| {
        window.update(cx, |_, window, cx| {
            editor.update(cx, |editor, cx| {
                let code = editor.editor().clone();
                code.update(cx, |code, cx| code.api(window, cx).select(at..at));
                then(editor, window, cx);
            });
        })
    })?;
    app.advance_clock(oxikube_ui::editor::COMPLETION_DEBOUNCE * 2);
    app.run_until_parked();
    Ok(())
}

/// A window holding a manifest editor with `text`, checked against the Deployment schema.
fn open(text: &str) -> Result<(ScreenshotApp, AnyWindowHandle, Entity<ManifestEditor>)> {
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
            text: text.to_owned(),
            schemas: Some(Rc::new(Schemas { cluster, port })),
            dispatcher: Rc::new(NoBus),
            resource: None,
            live: None,
        };
        let entity = cx.new(|cx| ManifestEditor::new(parts, window, cx));
        editor = Some(entity.clone());
        cx.new(|cx| Root::new(entity, window, cx))
    })?;
    Ok((app, window, editor.expect("built")))
}

/// An editor opened on `object` (read through a fake reader), settled and captured.
fn render_opened(object: Resource, mode: OpenMode, errors: usize) -> Result<RgbaImage> {
    let mut app = ScreenshotApp::with_assets(Arc::new(oxikube_ui::Assets));
    app.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
    });
    let cluster = ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"));
    let reader = Arc::new(FakeResourcePort::new());
    reader.insert(object.clone());
    let target = ResourceRef::new(
        cluster.clone(),
        object.kind.clone(),
        object.meta.namespace.clone(),
        object.name(),
    );
    let mut editor: Option<Entity<ManifestEditor>> = None;
    let window = app.open_window(size(px(SIZE.0 as f32), px(SIZE.1 as f32)), |window, cx| {
        let reader: Arc<dyn ResourceReader> = reader;
        let parts = ManifestEditorParts {
            title: "".into(),
            text: String::new(),
            schemas: Some(Rc::new(Schemas {
                cluster,
                port: Arc::new(FakeSchemaPort::new()),
            })),
            dispatcher: Rc::new(NoBus),
            resource: Some(ResourceOpen {
                target,
                mode,
                reader: Some(reader),
                cluster_read_only: false,
            }),
            live: None,
        };
        let entity = cx.new(|cx| ManifestEditor::new(parts, window, cx));
        editor = Some(entity.clone());
        cx.new(|cx| Root::new(entity, window, cx))
    })?;
    app.run_until_parked();
    app.advance_clock(VALIDATION_DEBOUNCE);
    app.run_until_parked();
    let editor = editor.expect("built");
    let found = app.update(|cx| editor.read(cx).model().problems().errors);
    anyhow::ensure!(found == errors, "{found} errors, wanted {errors}");
    app.capture(window)
}

fn render_open_deployment_dark() -> Result<RgbaImage> {
    let object = Resource::from_json(json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": "web",
            "namespace": "default",
            "labels": {"app": "web"},
            "managedFields": [{"manager": "kubectl", "operation": "Apply"}]
        },
        "spec": {
            "replicas": 2,
            "selector": {"matchLabels": {"app": "web"}},
            "template": {"spec": {"containers": [{"name": "web", "image": "nginx:1.27"}]}}
        },
        "status": {"availableReplicas": 2, "readyReplicas": 2, "replicas": 2}
    }))?;
    render_opened(object, OpenMode::View, 0)
}

fn render_open_secret_dark() -> Result<RgbaImage> {
    let object = Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "type": "Opaque",
        "metadata": {"name": "db", "namespace": "default"},
        "data": {"password": "c2VjcmV0LXBhc3N3b3Jk", "username": "YWRtaW4="}
    }))?;
    render_opened(object, OpenMode::Edit, 2)
}

const COMMENTED: &str = "\
# The web tier.
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web   # the name
  labels:
    app: web
spec:
  replicas: 3
  paused: false
";

/// A commented manifest converted to JSON (E10-S10): the JSON grammar and the language chip.
fn render_json_dark() -> Result<RgbaImage> {
    let mut app = ScreenshotApp::with_assets(Arc::new(oxikube_ui::Assets));
    app.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
    });
    let mut editor: Option<Entity<ManifestEditor>> = None;
    let window = app.open_window(size(px(SIZE.0 as f32), px(SIZE.1 as f32)), |window, cx| {
        let parts = ManifestEditorParts {
            title: "deployment".into(),
            text: COMMENTED.to_owned(),
            schemas: None,
            dispatcher: Rc::new(NoBus),
            resource: None,
            live: None,
        };
        let entity = cx.new(|cx| ManifestEditor::new(parts, window, cx));
        editor = Some(entity.clone());
        cx.new(|cx| Root::new(entity, window, cx))
    })?;
    let editor = editor.expect("built");
    app.run_until_parked();
    app.update(|cx| {
        window.update(cx, |_, window, cx| {
            editor.update(cx, |editor, cx| {
                editor.convert_to(Language::Json, window, cx)
            });
        })
    })?;
    app.run_until_parked();
    app.advance_clock(VALIDATION_DEBOUNCE);
    app.run_until_parked();
    let language = app.update(|cx| editor.read(cx).language());
    anyhow::ensure!(language == Language::Json, "{language}");
    app.capture(window)
}

fn main() -> ExitCode {
    run_golden_cases(
        env!("CARGO_MANIFEST_DIR"),
        &[
            GoldenCase {
                name: "manifest_editor_error_dark",
                size: SIZE,
                render: render_error_dark,
            },
            GoldenCase {
                name: "manifest_editor_open_deployment_dark",
                size: SIZE,
                render: render_open_deployment_dark,
            },
            GoldenCase {
                name: "manifest_editor_open_secret_dark",
                size: SIZE,
                render: render_open_secret_dark,
            },
            GoldenCase {
                name: "manifest_editor_json_dark",
                size: SIZE,
                render: render_json_dark,
            },
            GoldenCase {
                name: "manifest_editor_completion_dark",
                size: SIZE,
                render: render_completion_dark,
            },
            GoldenCase {
                name: "manifest_editor_hover_dark",
                size: SIZE,
                render: render_hover_dark,
            },
        ],
    )
}
