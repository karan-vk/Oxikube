//! Screenshots of create from template and multi-document apply (E10-S09), rendered through the
//! headless renderer:
//!
//! - `template_picker_dark`: the "Create resource" picker filtered by `dep`, a user template
//!   among the embedded ones, and the footer naming the folder of one's own templates.
//! - `batch_preview_dark`: the combined preview of four documents - a namespace to create, a
//!   ConfigMap to change (its diff below), an unchanged Secret, a Deployment the server refused.
//!
//! `harness = false` (the platform text system needs the process main thread); needs a GPU, so it
//! only builds with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_editor --features screenshot --test create_screenshot`.
//! Regenerate the goldens with `OXIKUBE_UPDATE_GOLDENS=1`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use gpui::{AppContext as _, WeakEntity, px, size};
use oxikube_app::{
    ApplyError, ApplyMethod, ApplyPreview, ApplyRequest, BatchPreview, DocumentPreview,
    PreviewOutcome, Template, TemplateCatalog,
};
use oxikube_domain::ids::{ClusterId, ContextName, ResourceRef};
use oxikube_editor::apply::preview_diff;
use oxikube_editor::templates::{TemplatePicker, TemplatePickerDelegate, embedded};
use oxikube_editor::view::{BatchPreviewDialog, BatchSession};
use oxikube_testkit::gpui_test::{GoldenCase, ScreenshotApp, run_golden_cases};
use oxikube_testkit::screenshot::RgbaImage;
use oxikube_ui::root::Root;
use serde_json::{Value, json};

fn app() -> ScreenshotApp {
    let mut app = ScreenshotApp::with_assets(Arc::new(oxikube_ui::Assets));
    app.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
    });
    app
}

fn render_picker() -> Result<RgbaImage> {
    let mut app = app();
    let mut templates = embedded();
    let mine = Template::from_user_file(
        "web",
        "# name: web\n# description: Our standard web Deployment\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: default\n",
    )
    .expect("a template");
    templates.push(mine);
    let catalog = TemplateCatalog::of(templates);
    let mut picker: Option<gpui::Entity<TemplatePicker>> = None;
    let window = app.open_window(size(px(640.), px(300.)), |window, cx| {
        let entity = cx.new(|cx| {
            TemplatePickerDelegate::picker(
                catalog,
                Some(PathBuf::from("~/.config/oxikube/templates")),
                Rc::new(|_, _, _| {}),
                window,
                cx,
            )
        });
        picker = Some(entity.clone());
        cx.new(|cx| Root::new(entity, window, cx))
    })?;
    let picker = picker.expect("built");
    app.update(|cx| {
        window.update(cx, |_, window, cx| {
            picker.update(cx, |picker, cx| picker.set_query("dep", window, cx));
        })
    })?;
    app.capture(window)
}

fn cluster() -> ClusterId {
    ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"))
}

fn target(kind: &str, namespace: Option<&str>, name: &str) -> ResourceRef {
    let gvk = oxikube_domain::ids::Gvk::new("", "v1", kind);
    match namespace {
        Some(ns) => ResourceRef::namespaced(cluster(), gvk, ns, name),
        None => ResourceRef::cluster_scoped(cluster(), gvk, name),
    }
}

fn object(kind: &str, namespace: Option<&str>, name: &str, data: Value) -> Value {
    let mut metadata = json!({"name": name});
    if let Some(ns) = namespace {
        metadata["namespace"] = json!(ns);
    }
    let mut object = json!({"apiVersion": "v1", "kind": kind, "metadata": metadata});
    if let Value::Object(fields) = data {
        for (key, value) in fields {
            object[key] = value;
        }
    }
    object
}

fn ready(
    index: usize,
    label: &str,
    target: ResourceRef,
    live: Option<Value>,
    result: Value,
) -> DocumentPreview {
    let request = ApplyRequest::new(cluster(), "kind: placeholder\n");
    DocumentPreview {
        index,
        label: label.to_owned(),
        request: request.clone(),
        depends_on: Vec::new(),
        outcome: PreviewOutcome::Ready(ApplyPreview {
            request,
            target,
            method: ApplyMethod::ServerSide,
            live,
            result,
            resource_version: Some("7".into()),
        }),
    }
}

fn render_batch() -> Result<RgbaImage> {
    let mut app = app();
    let live_config = object(
        "ConfigMap",
        Some("shop"),
        "web-config",
        json!({"data": {"log_level": "info", "port": "8080", "region": "eu-west-1"}}),
    );
    let new_config = object(
        "ConfigMap",
        Some("shop"),
        "web-config",
        json!({"data": {"log_level": "debug", "port": "8080", "region": "eu-west-1"}}),
    );
    let secret = object(
        "Secret",
        Some("shop"),
        "db",
        json!({"stringData": {"password": "(value #1a2b3c4d)"}}),
    );
    let refused = DocumentPreview {
        index: 3,
        label: "Deployment web".to_owned(),
        request: ApplyRequest::new(cluster(), "kind: placeholder\n"),
        depends_on: Vec::new(),
        outcome: PreviewOutcome::Failed(ApplyError::Invalid {
            message: "Deployment.apps \"web\" is invalid: spec.selector: Required value".into(),
            causes: Vec::new(),
        }),
    };
    let preview = BatchPreview {
        documents: vec![
            ready(
                0,
                "Namespace shop",
                target("Namespace", None, "shop"),
                None,
                object("Namespace", None, "shop", json!({})),
            ),
            ready(
                1,
                "ConfigMap shop/web-config",
                target("ConfigMap", Some("shop"), "web-config"),
                Some(live_config),
                new_config,
            ),
            ready(
                2,
                "Secret shop/db",
                target("Secret", Some("shop"), "db"),
                Some(secret.clone()),
                secret,
            ),
            refused,
        ],
    };
    let diffs = preview
        .documents
        .iter()
        .map(|d| d.preview().map(|p| Arc::new(preview_diff(p))))
        .collect();
    let session = BatchSession {
        text: "".into(),
        ranges: vec![0..0; 4],
        version: 1,
    };
    let window = app.open_window(size(px(900.), px(640.)), |window, cx| {
        let entity = cx.new(|cx| {
            let mut dialog =
                BatchPreviewDialog::new(preview, diffs, session, WeakEntity::new_invalid(), cx);
            dialog.select(1, cx);
            dialog
        });
        cx.new(|cx| Root::new(entity, window, cx))
    })?;
    app.capture(window)
}

fn main() -> ExitCode {
    run_golden_cases(
        env!("CARGO_MANIFEST_DIR"),
        &[
            GoldenCase {
                name: "template_picker_dark",
                size: (640, 300),
                render: render_picker,
            },
            GoldenCase {
                name: "batch_preview_dark",
                size: (900, 640),
                render: render_batch,
            },
        ],
    )
}
