//! Screenshots of the cross-cluster resource diff (E20-S09), rendered through the headless
//! renderer:
//!
//! - `resource_diff_side_by_side_dark`: a ConfigMap of staging against production: the
//!   changed-path summary above the side-by-side line diff, only the changed lines highlighted.
//! - `resource_diff_secret_dark`: the same for a Secret: `changed` / `same` per key and no value.
//! - `resource_diff_missing_dark`: one side does not exist, the other reads as all added.
//!
//! `harness = false` (the platform text system needs the process main thread); needs a GPU, so it
//! only builds with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_editor --features screenshot --test resource_diff_screenshot`.
//! Regenerate the goldens with `OXIKUBE_UPDATE_GOLDENS=1`.

use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use gpui::{App, AppContext as _, px, size};
use oxikube_app::diff::{DiffOptions, DiffRequest, IgnorePaths, compare};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_editor::view::{DiffData, DiffState, ResourceDiffView};
use oxikube_testkit::gpui_test::{GoldenCase, ScreenshotApp, run_golden_cases};
use oxikube_testkit::screenshot::RgbaImage;
use oxikube_ui::root::Root;
use oxikube_workspace::CommandDispatcher;
use serde_json::{Value, json};

const SIZE: (u32, u32) = (1000, 560);

struct NoBus;

impl CommandDispatcher for NoBus {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

fn reference(cluster: &str, gvk: Gvk, namespace: &str) -> ResourceRef {
    let name = ContextName::new(cluster);
    ResourceRef::namespaced(ClusterId::new("/kubeconfig", &name), gvk, namespace, "web")
}

fn object(kind: &str, namespace: &str, body: Value) -> Resource {
    let mut value = json!({
        "apiVersion": "v1", "kind": kind,
        "metadata": {"name": "web", "namespace": namespace, "uid": "u", "resourceVersion": "9"}
    });
    value
        .as_object_mut()
        .expect("an object")
        .extend(body.as_object().expect("an object").clone());
    Resource::from_json(value).expect("a valid resource")
}

fn app() -> ScreenshotApp {
    let mut app = ScreenshotApp::with_assets(Arc::new(oxikube_ui::Assets));
    app.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
    });
    app
}

fn render(gvk: Gvk, left: Option<Resource>, right: Option<Resource>) -> Result<RgbaImage> {
    let request = DiffRequest::new(
        reference("staging", gvk.clone(), "shop"),
        reference("production", gvk, "shop"),
    );
    let options = DiffOptions {
        ignore: IgnorePaths::standard(),
        ..DiffOptions::default()
    };
    let mut diff = compare(&request, left, right, &options);
    diff.left.cluster_title = "staging".to_owned();
    diff.right.cluster_title = "production".to_owned();
    let data = DiffData::new(diff);
    let mut app = app();
    let window = app.open_window(size(px(SIZE.0 as f32), px(SIZE.1 as f32)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = ResourceDiffView::new(request.clone(), Rc::new(NoBus), cx);
            view.update_state(request.clone(), DiffState::Ready(Arc::new(data)), cx);
            view
        });
        cx.new(|cx| Root::new(view, window, cx))
    })?;
    app.run_until_parked();
    app.capture(window)
}

fn config_maps() -> Result<RgbaImage> {
    let data = |url: &str, replicas: &str| {
        json!({"data": {
            "api_url": url, "log_level": "info", "region": "eu-west-1",
            "replicas": replicas, "timeout": "30s"
        }})
    };
    render(
        Gvk::new("", "v1", "ConfigMap"),
        Some(object(
            "ConfigMap",
            "shop",
            data("https://stg.example.com", "2"),
        )),
        Some(object(
            "ConfigMap",
            "shop",
            data("https://example.com", "6"),
        )),
    )
}

fn secrets() -> Result<RgbaImage> {
    let data = |password: &str| json!({"data": {"password": password, "username": "d2Vi"}, "type": "Opaque"});
    render(
        Gvk::new("", "v1", "Secret"),
        Some(object("Secret", "shop", data("c3RhZ2luZw=="))),
        Some(object("Secret", "shop", data("cHJvZHVjdGlvbg=="))),
    )
}

fn missing() -> Result<RgbaImage> {
    render(
        Gvk::new("", "v1", "ConfigMap"),
        None,
        Some(object(
            "ConfigMap",
            "shop",
            json!({"data": {"mode": "blue"}}),
        )),
    )
}

fn main() -> ExitCode {
    run_golden_cases(
        env!("CARGO_MANIFEST_DIR"),
        &[
            GoldenCase {
                name: "resource_diff_side_by_side_dark",
                size: SIZE,
                render: config_maps,
            },
            GoldenCase {
                name: "resource_diff_secret_dark",
                size: SIZE,
                render: secrets,
            },
            GoldenCase {
                name: "resource_diff_missing_dark",
                size: SIZE,
                render: missing,
            },
        ],
    )
}
