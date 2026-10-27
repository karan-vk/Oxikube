//! Screenshots of diff vs live (E10-S07), rendered through the headless renderer:
//!
//! - `manifest_banner_dark`: a ConfigMap loaded from the server, edited (a comment typed at the
//!   top), then changed on the server: the "changed on the server" banner with Merge, Reload and
//!   Keep mine over the buffer.
//! - `manifest_diff_side_by_side_dark` / `manifest_diff_inline_dark`: a typed Deployment manifest
//!   against the live one (replicas and image differ), the diff in the bottom panel side by side
//!   and inline.
//!
//! The live object comes from a `FakeResourcePort` on the deterministic runtime. `harness = false`
//! (the platform text system needs the process main thread); needs a GPU, so it only builds with
//! `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_editor --features screenshot --test diff_screenshot`.
//! Regenerate the goldens with `OXIKUBE_UPDATE_GOLDENS=1`.

use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{Result, ensure};
use gpui::{App, AppContext as _, Entity, Focusable as _, px, size};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_editor::diff::SyncState;
use oxikube_editor::view::{
    DIFF_DEBOUNCE, ManifestEditor, ManifestEditorParts, VALIDATION_DEBOUNCE,
};
use oxikube_testkit::fakes::FakeResourcePort;
use oxikube_testkit::gpui_test::{GoldenCase, ScreenshotApp, run_golden_cases};
use oxikube_testkit::screenshot::RgbaImage;
use oxikube_testkit::{ScriptedFeed, TICK};
use oxikube_ui::root::Root;
use oxikube_workspace::CommandDispatcher;
use serde_json::json;

mod support;

use support::FakeLive;

const SIZE: (u32, u32) = (900, 520);

struct NoBus;

impl CommandDispatcher for NoBus {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

fn config_map(rv: &str, level: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {"name": "web-config", "namespace": "shop", "resourceVersion": rv},
        "data": {"log_level": level, "port": "8080", "region": "eu-west-1"}
    }))
    .expect("the ConfigMap fixture is a valid resource")
}

fn deployment() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {"name": "web", "namespace": "shop", "resourceVersion": "41",
                     "managedFields": [{"manager": "kubectl"}]},
        "spec": {
            "replicas": 3,
            "selector": {"matchLabels": {"app": "web"}},
            "template": {
                "metadata": {"labels": {"app": "web"}},
                "spec": {"containers": [{"name": "web", "image": "nginx:1.27",
                                         "ports": [{"containerPort": 80}]}]}
            }
        },
        "status": {"replicas": 3, "readyReplicas": 3}
    }))
    .expect("the Deployment fixture is a valid resource")
}

const TYPED: &str = "\
# The shop's front end.
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
  namespace: shop
spec:
  replicas: 5
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
          image: nginx:1.28
          ports:
            - containerPort: 80
";

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

fn cluster() -> ClusterId {
    ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"))
}

/// A window with an editor holding `text`, following `port`'s objects; `setup` runs with the
/// window once the editor exists.
fn editor_window(
    app: &mut ScreenshotApp,
    title: &str,
    text: &str,
    port: Arc<FakeResourcePort>,
    setup: impl FnOnce(&mut ManifestEditor, &mut gpui::Window, &mut gpui::Context<ManifestEditor>),
) -> Result<(gpui::AnyWindowHandle, Entity<ManifestEditor>)> {
    let mut editor: Option<Entity<ManifestEditor>> = None;
    let window = app.open_window(size(px(SIZE.0 as f32), px(SIZE.1 as f32)), |window, cx| {
        let parts = ManifestEditorParts {
            title: title.to_owned().into(),
            text: text.to_owned(),
            schemas: None,
            dispatcher: Rc::new(NoBus),
            resource: None,
            live: Some(Rc::new(FakeLive::new(cluster(), port))),
        };
        let entity = cx.new(|cx| ManifestEditor::new(parts, window, cx));
        entity.update(cx, |editor, cx| {
            window.focus(&editor.focus_handle(cx), cx);
            setup(editor, window, cx);
        });
        editor = Some(entity.clone());
        cx.new(|cx| Root::new(entity, window, cx))
    })?;
    app.advance_clock(VALIDATION_DEBOUNCE.max(DIFF_DEBOUNCE));
    Ok((window, editor.expect("built")))
}

fn render_banner() -> Result<RgbaImage> {
    let mut app = app();
    let port = Arc::new(FakeResourcePort::new());
    ScriptedFeed::new()
        .initial([config_map("1", "info")])
        .modify(1, config_map("2", "debug"))
        .install(&port);
    let (window, editor) = editor_window(&mut app, "web-config", "", port.clone(), |e, w, cx| {
        e.follow_object(config_map("1", "info"), None, w, cx);
    })?;
    app.simulate_keystrokes(window, "# space n o t e enter")?;
    port.clock().advance(TICK);
    app.run_until_parked();
    app.advance_clock(TICK);
    let state = app.update(|cx| editor.read(cx).live_sync().state());
    ensure!(state == SyncState::Banner, "{state:?}");
    app.capture(window)
}

fn render_diff(inline: bool) -> Result<RgbaImage> {
    let mut app = app();
    let port = Arc::new(FakeResourcePort::new());
    ScriptedFeed::new().initial([deployment()]).install(&port);
    let (window, editor) = editor_window(&mut app, "web.yaml", TYPED, port, |e, w, cx| {
        e.toggle_diff(w, cx);
        if inline {
            e.toggle_diff_layout(cx);
        }
    })?;
    app.advance_clock(TICK);
    let hunks = app.update(|cx| editor.read(cx).diff().map(|d| d.hunks()));
    ensure!(hunks.is_some_and(|h| h > 0), "{hunks:?}");
    app.capture(window)
}

fn main() -> ExitCode {
    run_golden_cases(
        env!("CARGO_MANIFEST_DIR"),
        &[
            GoldenCase {
                name: "manifest_banner_dark",
                size: SIZE,
                render: render_banner,
            },
            GoldenCase {
                name: "manifest_diff_side_by_side_dark",
                size: SIZE,
                render: || render_diff(false),
            },
            GoldenCase {
                name: "manifest_diff_inline_dark",
                size: SIZE,
                render: || render_diff(true),
            },
        ],
    )
}
