//! The Role rule dialog, rendered through `Window::render_to_image` (E17-S05).
//!
//! - `role_dialog_edit_dark`: a Role with two rules, verb checkboxes, the resource inputs and a
//!   warning (resource names do not narrow `list`).
//! - `role_dialog_aggregated_light`: an aggregated ClusterRole, whose rules are read-only.
//!
//! `harness = false` and `--features screenshot`, like the other screenshot tests. Checks the
//! image size, that it is not blank and, when a golden exists for this OS, that it matches within
//! tolerance. Regenerate with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a harness-less test binary reports its results on stdout and stderr"
)]

use std::cell::RefCell;
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use futures::FutureExt as _;
use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::ClusterSessionManager;
use oxikube_app::session::SessionOptions;
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_rbac_ui::role::{DialogMode, RoleDialog, RoleDialogDeps};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::kinds::core_kinds;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_testkit::{FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort};
use oxikube_workspace::CommandDispatcher;
use serde_json::json;

const WIDTH: f32 = 860.0;
const HEIGHT: f32 = 720.0;

struct Nowhere;

impl CommandDispatcher for Nowhere {
    fn dispatch(&self, _: Command, _: &mut gpui::App) {}
}

#[derive(Clone, Copy, PartialEq)]
enum Scene {
    EditDark,
    AggregatedLight,
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn context() -> ClusterContext {
    let name = ContextName::new("kind-oxikube");
    ClusterContext::new(
        ClusterId::new("/home/me/.kube/config", &name),
        name,
        SourceId("kubeconfig".into()),
    )
}

fn object(value: serde_json::Value) -> Resource {
    Resource::from_json(value).expect("a valid object")
}

fn sessions() -> ClusterSessionManager {
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([context()]));
    let clock = Arc::new(FakeClockPort::default());
    let ports = connector.ports_for(&context().cluster);
    ports.discovery.set_kinds(core_kinds());
    ports.resources.insert(object(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "Role",
        "metadata": {"name": "app-reader", "namespace": "shop", "resourceVersion": "12"},
        "rules": [
            {"apiGroups": ["apps"], "resources": ["deployments", "deployments/scale"],
             "verbs": ["get", "list", "watch"]},
            {"apiGroups": [""], "resources": ["configmaps"], "resourceNames": ["app-config"],
             "verbs": ["get", "list", "update"]},
        ],
    })));
    ports.resources.insert(object(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
        "metadata": {"name": "monitoring", "resourceVersion": "4"},
        "aggregationRule": {"clusterRoleSelectors": [
            {"matchLabels": {"rbac.example.com/aggregate-to-monitoring": "true"}}]},
        "rules": [{"apiGroups": [""], "resources": ["pods", "nodes"], "verbs": ["get", "list", "watch"]}],
    })));
    let sessions = ClusterSessionManager::new(connector, source, clock);
    sessions.open(&context(), SessionOptions::default());
    sessions
        .connect(&context().cluster)
        .now_or_never()
        .expect("the fake connects at once")
        .expect("connected");
    sessions
}

fn render(scene: Scene) -> anyhow::Result<RgbaImage> {
    let sessions = sessions();
    let target = match scene {
        Scene::EditDark => ResourceRef::namespaced(
            context().cluster,
            Gvk::new("rbac.authorization.k8s.io", "v1", "Role"),
            "shop",
            "app-reader",
        ),
        Scene::AggregatedLight => ResourceRef::cluster_scoped(
            context().cluster,
            Gvk::new("rbac.authorization.k8s.io", "v1", "ClusterRole"),
            "monitoring",
        ),
    };
    let held: Rc<RefCell<Option<gpui::Entity<RoleDialog>>>> = Rc::default();
    let hold = held.clone();
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        let tokens = if scene == Scene::AggregatedLight {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let deps = RoleDialogDeps {
            sessions,
            dispatcher: Rc::new(Nowhere),
            cluster: context().cluster,
        };
        let view = cx.new(|cx| RoleDialog::new(deps, DialogMode::Edit(target), window, cx));
        *hold.borrow_mut() = Some(view.clone());
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    cx.run_until_parked();
    for _ in 0..2 {
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
        cx.run_until_parked();
    }
    let shot = cx.capture_screenshot(window.into());
    held.borrow_mut().take();
    shot
}

fn check(name: &str, image: RgbaImage) -> anyhow::Result<()> {
    let scale = HEADLESS_SCALE_FACTOR;
    anyhow::ensure!(
        image.dimensions() == (WIDTH as u32 * scale, HEIGHT as u32 * scale),
        "unexpected image size {:?}",
        image.dimensions()
    );
    anyhow::ensure!(
        distinct_colors_at_least(&image, 8),
        "{name}: frame looks blank"
    );
    let goldens = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    let golden = golden_path(&goldens, name);
    let updating =
        std::env::var_os("OXIKUBE_UPDATE_GOLDENS").is_some_and(|v| v != "0" && !v.is_empty());
    if golden.exists() || updating {
        check_golden(&image, &golden, Tolerance::default())?;
        println!("{name} matches {}", golden.display());
    } else {
        println!(
            "no golden for {} yet; structural checks only",
            std::env::consts::OS
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    for (name, scene) in [
        ("role_dialog_edit_dark", Scene::EditDark),
        ("role_dialog_aggregated_light", Scene::AggregatedLight),
    ] {
        if let Err(err) = render(scene).and_then(|image| check(name, image)) {
            eprintln!("{name} screenshot failed: {err:#}");
            return ExitCode::FAILURE;
        }
    }
    println!("role dialog screenshots: ok");
    ExitCode::SUCCESS
}
