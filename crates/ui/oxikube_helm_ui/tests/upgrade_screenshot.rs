//! The Helm upgrade tab, rendered through `Window::render_to_image` (E16-S10).
//!
//! - `upgrade_values_diff_dark`: the release's values prefilled and edited, the values diff beside
//!   them (a password shown as a fingerprint, never the value).
//! - `upgrade_preview_dark`: the preview (objects added, changed, removed and the rendered
//!   manifest).
//! - `upgrade_confirm_light`: the guard's medium-risk confirmation after the preview.
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
use std::time::Duration;

use futures::FutureExt as _;
use gpui::{AppContext as _, HeadlessAppContext, px, size};
use jiff::Timestamp;
use oxikube_app::command_bus::CommandRegistry;
use oxikube_app::session::SessionOptions;
use oxikube_app::{
    ChartsService, ClusterSessionManager, CommandBus, HelmReposService, MutationGuard,
    ReleaseProgress, UpgradeFlow, UpgradeRequests,
};
use oxikube_domain::helm::{
    ChartHit, ChartRef, ReleaseRef, ReleaseStatus, RepoEntry, Revision, Values,
};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_helm_ui::upgrade::{FlowBackend, HelmUpgradeView, UpgradeDeps};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeHelmPort, FakeStatePort,
};

const WIDTH: f32 = 1280.0;
const HEIGHT: f32 = 680.0;

const CURRENT_VALUES: &str = "replicaCount: 2\nimage:\n  tag: 1.25.1\nservice:\n  type: ClusterIP\nauth:\n  password: hunter2-value\n";

const EDITED_VALUES: &str = "replicaCount: 3\nimage:\n  tag: 1.25.3\nservice:\n  type: ClusterIP\nauth:\n  password: hunter3-value\n";

const DEFAULTS_OLD: &str = "# -- Number of replicas\nreplicaCount: 1\nservice:\n  port: 80\n";

const DEFAULTS_NEW: &str = "# -- Number of replicas\nreplicaCount: 1\nservice:\n  httpPort: 80\ningress:\n  enabled: false\n";

const CURRENT_MANIFEST: &str = "---\napiVersion: v1\nkind: Service\nmetadata:\n  name: web-nginx\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web-nginx\nspec:\n  replicas: 2\n---\napiVersion: v1\nkind: Secret\nmetadata:\n  name: web-nginx-auth\ndata:\n  password: b2xkcGFzcw==\n";

const RENDERED: &str = "---\napiVersion: v1\nkind: Service\nmetadata:\n  name: web-nginx\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web-nginx\nspec:\n  replicas: 3\n---\napiVersion: networking.k8s.io/v1\nkind: Ingress\nmetadata:\n  name: web-nginx\n---\napiVersion: v1\nkind: Secret\nmetadata:\n  name: web-nginx-extra\ndata:\n  token: c3VwZXJzZWNyZXQ=\n";

#[derive(Clone, Copy, PartialEq)]
enum Scene {
    ValuesDiffDark,
    PreviewDark,
    ConfirmLight,
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

fn release() -> ReleaseRef {
    ReleaseRef::new(context().cluster, "default", "web")
}

fn revision() -> Revision {
    Revision {
        revision: 3,
        status: ReleaseStatus::Deployed,
        chart: ChartRef::new("nginx", "15.2.0"),
        app_version: Some("1.25.1".into()),
        updated: Timestamp::UNIX_EPOCH,
        description: String::new(),
    }
}

fn backend(fake: &Arc<FakeHelmPort>) -> FlowBackend {
    let script = fake.script();
    script.repos.push_ok(vec![RepoEntry {
        name: "bitnami".into(),
        url: "https://charts.bitnami.com/bitnami".into(),
    }]);
    script.history.push_ok(vec![revision()]);
    script
        .values
        .push_ok(Values::from_yaml(CURRENT_VALUES).expect("values"));
    script.search.push_ok(
        ["15.4.0", "15.3.1", "15.2.0"]
            .into_iter()
            .map(|v| ChartHit {
                chart: ChartRef::new("nginx", v).in_repo("bitnami"),
                app_version: Some("1.25.3".into()),
                description: "NGINX".into(),
            })
            .collect(),
    );
    script.show.push_ok(DEFAULTS_OLD.to_owned());
    script.show.push_ok(DEFAULTS_NEW.to_owned());
    script.manifest.push_ok(CURRENT_MANIFEST.to_owned());
    script.history.push_ok(vec![revision()]);
    script.template.push_ok(RENDERED.to_owned());
    script.upgrade.push_ok(Vec::new());
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([context()]));
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    sessions.open(&context(), SessionOptions::default());
    sessions
        .connect(&context().cluster)
        .now_or_never()
        .expect("the fake connects at once")
        .expect("connected");
    let progress = ReleaseProgress::new();
    let mut registry = CommandRegistry::new();
    let (port, follow) = (fake.clone(), progress.clone());
    registry
        .install("oxikube_app::helm_upgrade", |r| {
            oxikube_app::helm_upgrade::register_commands(r, port, follow, UpgradeRequests::new())
        })
        .expect("registers");
    let guard = MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock.clone());
    let flow = UpgradeFlow::new(
        CommandBus::new(registry, guard),
        fake.clone(),
        progress,
        "me",
    );
    let repos = HelmReposService::new(fake.clone(), fake.clone(), clock);
    FlowBackend::new(
        flow,
        ChartsService::new(fake.clone(), repos.catalog_changes()),
    )
}

fn render(scene: Scene) -> anyhow::Result<RgbaImage> {
    let fake = Arc::new(FakeHelmPort::new());
    let backend = backend(&fake);
    let held: Rc<RefCell<Option<gpui::Entity<HelmUpgradeView>>>> = Rc::default();
    let hold = held.clone();
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        let tokens = if scene == Scene::ConfirmLight {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let deps = UpgradeDeps {
            backend: Rc::new(backend),
            cluster: context().cluster,
            cluster_label: "kind-oxikube".into(),
        };
        let view = cx.new(|cx| HelmUpgradeView::new(deps, Some(release()), window, cx));
        *hold.borrow_mut() = Some(view.clone());
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    cx.run_until_parked();
    let view = held.borrow().clone().expect("the upgrade view");
    let step = |cx: &mut HeadlessAppContext,
                f: &dyn Fn(
        &mut HelmUpgradeView,
        &mut gpui::Window,
        &mut gpui::Context<HelmUpgradeView>,
    )|
     -> anyhow::Result<()> {
        cx.update_window(window.into(), |_, window, cx| {
            view.update(cx, |view, cx| f(view, window, cx));
        })?;
        for _ in 0..3 {
            cx.run_until_parked();
            cx.advance_clock(Duration::from_millis(200));
        }
        cx.run_until_parked();
        Ok(())
    };
    step(&mut cx, &|view, window, cx| {
        view.set_values(EDITED_VALUES, window, cx);
    })?;
    if scene != Scene::ValuesDiffDark {
        step(&mut cx, &|view, window, cx| view.preview(window, cx))?;
    }
    if scene == Scene::ConfirmLight {
        step(&mut cx, &|view, _, cx| view.upgrade(cx))?;
    }
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    let shot = cx.capture_screenshot(window.into());
    held.borrow_mut().take();
    drop(view);
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
        ("upgrade_values_diff_dark", Scene::ValuesDiffDark),
        ("upgrade_preview_dark", Scene::PreviewDark),
        ("upgrade_confirm_light", Scene::ConfirmLight),
    ] {
        if let Err(err) = render(scene).and_then(|image| check(name, image)) {
            eprintln!("{name} screenshot failed: {err:#}");
            return ExitCode::FAILURE;
        }
    }
    println!("helm upgrade screenshots: ok");
    ExitCode::SUCCESS
}
