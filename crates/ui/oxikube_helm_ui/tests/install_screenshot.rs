//! The Helm install tab, rendered through `Window::render_to_image` (E16-S09).
//!
//! - `install_preview_dark`: the form, the chart's default values in the editor, and the preview
//!   (objects per kind, the rendered manifest with a Secret's data masked).
//! - `install_confirm_light`: the guard's low-risk confirmation after the preview.
//! - `install_log`: the install's output streamed into the log.
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
use oxikube_app::command_bus::CommandRegistry;
use oxikube_app::session::SessionOptions;
use oxikube_app::{
    ChartsService, ClusterSessionManager, CommandBus, HelmReposService, InstallFlow, MutationGuard,
    ReleaseProgress,
};
use oxikube_domain::helm::{ChartHit, ChartRef, OutputLine, RepoEntry};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_helm_ui::install::{FlowBackend, HelmInstallView, InstallDeps};
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

const DEFAULTS: &str = "# Default values for nginx.\n\n# -- Number of replicas\nreplicaCount: 1\n\nimage:\n  # -- Image registry\n  registry: docker.io\n  # -- Image repository\n  repository: bitnami/nginx\n  tag: 1.25.3\n\nservice:\n  # -- How the Service is exposed\n  type: LoadBalancer\n  port: 80\n\nauth:\n  # -- The admin password\n  password: \"\"\n";

const RENDERED: &str = "---\n# Source: nginx/templates/svc.yaml\napiVersion: v1\nkind: Service\nmetadata:\n  name: web-nginx\nspec:\n  type: LoadBalancer\n  ports:\n    - port: 80\n---\n# Source: nginx/templates/secret.yaml\napiVersion: v1\nkind: Secret\nmetadata:\n  name: web-nginx-auth\ndata:\n  password: c3VwZXJzZWNyZXQ=\n---\n# Source: nginx/templates/deployment.yaml\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web-nginx\nspec:\n  replicas: 2\n";

#[derive(Clone, Copy, PartialEq)]
enum Scene {
    PreviewDark,
    ConfirmLight,
    Log,
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

fn backend(fake: &Arc<FakeHelmPort>) -> FlowBackend {
    fake.script().repos.push_ok(vec![RepoEntry {
        name: "bitnami".into(),
        url: "https://charts.bitnami.com/bitnami".into(),
    }]);
    fake.script().show.push_ok(DEFAULTS.to_owned());
    fake.script().search.push_ok(
        ["15.4.0", "15.3.1", "15.2.0"]
            .into_iter()
            .map(|v| ChartHit {
                chart: ChartRef::new("nginx", v).in_repo("bitnami"),
                app_version: Some("1.25.3".into()),
                description: "NGINX".into(),
            })
            .collect(),
    );
    fake.script().template.push_ok(RENDERED.to_owned());
    fake.script().install.push_ok(Vec::new());
    fake.script().install.push_ok(vec![
        OutputLine::stdout("NAME: web"),
        OutputLine::stdout("LAST DEPLOYED: Wed Oct 21 10:12:03 2026"),
        OutputLine::stdout("NAMESPACE: default"),
        OutputLine::stdout("STATUS: deployed"),
        OutputLine::stdout("REVISION: 1"),
        OutputLine::stderr("W1021 10:12:02 warnings.go:70] spec.template uses a deprecated field"),
        OutputLine::stdout("NOTES:"),
        OutputLine::stdout("** Please be patient while the chart is being deployed **"),
    ]);
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
        .install("oxikube_app::helm_install", |r| {
            oxikube_app::helm_install::register_commands(r, port, follow)
        })
        .expect("registers");
    let guard = MutationGuard::new(
        sessions.clone(),
        Arc::new(FakeStatePort::new()),
        clock.clone(),
    );
    let flow = InstallFlow::new(CommandBus::new(registry, guard), sessions, progress, "me");
    let repos = HelmReposService::new(fake.clone(), fake.clone(), clock);
    FlowBackend::new(
        flow,
        ChartsService::new(fake.clone(), repos.catalog_changes()),
    )
}

fn render(scene: Scene) -> anyhow::Result<RgbaImage> {
    let fake = Arc::new(FakeHelmPort::new());
    let backend = backend(&fake);
    let held: Rc<RefCell<Option<gpui::Entity<HelmInstallView>>>> = Rc::default();
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
        let deps = InstallDeps {
            backend: Rc::new(backend),
            cluster: context().cluster,
            cluster_label: "kind-oxikube".into(),
        };
        let chart = ChartRef::new("nginx", "15.4.0").in_repo("bitnami");
        let view = cx.new(|cx| HelmInstallView::new(deps, chart, window, cx));
        *hold.borrow_mut() = Some(view.clone());
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    cx.run_until_parked();
    let view = held.borrow().clone().expect("the install view");
    let step = |cx: &mut HeadlessAppContext,
                f: &dyn Fn(
        &mut HelmInstallView,
        &mut gpui::Window,
        &mut gpui::Context<HelmInstallView>,
    )|
     -> anyhow::Result<()> {
        cx.update_window(window.into(), |_, window, cx| {
            view.update(cx, |view, cx| f(view, window, cx));
        })?;
        cx.run_until_parked();
        Ok(())
    };
    step(&mut cx, &|view, _, cx| view.set_create_namespace(true, cx))?;
    step(&mut cx, &|view, window, cx| view.preview(window, cx))?;
    if scene != Scene::PreviewDark {
        step(&mut cx, &|view, _, cx| view.install(cx))?;
    }
    if scene == Scene::Log {
        step(&mut cx, &|view, _, cx| view.confirm(cx))?;
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
        ("install_preview_dark", Scene::PreviewDark),
        ("install_confirm_light", Scene::ConfirmLight),
        ("install_log", Scene::Log),
    ] {
        if let Err(err) = render(scene).and_then(|image| check(name, image)) {
            eprintln!("{name} screenshot failed: {err:#}");
            return ExitCode::FAILURE;
        }
    }
    println!("helm install screenshots: ok");
    ExitCode::SUCCESS
}
