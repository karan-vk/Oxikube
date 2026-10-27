//! The Helm charts catalog, rendered through `Window::render_to_image` (E16-S08).
//!
//! - `charts_list`: chart cards (icon placeholder, name, repository, description, versions).
//! - `charts_readme_dark`, `charts_readme_light`: a chart selected, its README rendered
//!   (headings, nested lists, a table, a code block, links; raw HTML and images stripped).
//! - `charts_values`: the default values tab.
//! - `charts_no_repos`: no repository configured: the default ones are offered.
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

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::{ChartsService, HelmReposService};
use oxikube_domain::command::Command;
use oxikube_domain::helm::{ChartHit, ChartRef, RepoEntry};
use oxikube_helm_ui::charts::{ChartsDeps, HelmChartsView, ServiceBackend, Tab};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_testkit::{FakeClockPort, FakeHelmPort};
use oxikube_workspace::CommandDispatcher;

const WIDTH: f32 = 1200.0;
const HEIGHT: f32 = 640.0;

const README: &str = "<h1 align=\"center\">nginx</h1>\n\n![badge](https://img.shields.io/x.svg)\n\n\
# NGINX Open Source\n\nNGINX is a web server that can also be used as a reverse proxy, load \
balancer, mail proxy and HTTP cache. See [the docs](https://nginx.org).\n\n## Installing\n\n\
```console\nhelm install my-release bitnami/nginx\n```\n\n## Parameters\n\n- Global\n  - \
`global.imageRegistry` the registry\n  - `global.storageClass` the storage class\n- Common\n\n\
| Name | Default | Description |\n|------|---------|-------------|\n| `replicaCount` | `1` | \
Replicas |\n| `service.type` | `LoadBalancer` | Service type |\n\n<script>alert(1)</script>\n";

const VALUES: &str = "# Default values for nginx.\nreplicaCount: 1\nimage:\n  registry: docker.io\n  \
repository: bitnami/nginx\n  tag: 1.25.0\nservice:\n  type: LoadBalancer\n  port: 80\nresources: {}\n";

#[derive(Clone, Copy)]
enum Scene {
    List,
    ReadmeDark,
    ReadmeLight,
    Values,
    NoRepos,
}

struct Ignore;

impl CommandDispatcher for Ignore {
    fn dispatch(&self, _: Command, _: &mut gpui::App) {}
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn hit(repo: &str, name: &str, version: &str, app: &str, description: &str) -> ChartHit {
    ChartHit {
        chart: ChartRef::new(name, version).in_repo(repo),
        app_version: Some(app.to_owned()),
        description: description.to_owned(),
    }
}

fn service(scene: Scene) -> ChartsService {
    let fake = Arc::new(FakeHelmPort::new());
    let repos = HelmReposService::new(
        fake.clone(),
        fake.clone(),
        Arc::new(FakeClockPort::default()),
    );
    let entries = match scene {
        Scene::NoRepos => Vec::new(),
        _ => vec![
            RepoEntry {
                name: "bitnami".into(),
                url: "https://charts.bitnami.com/bitnami".into(),
            },
            RepoEntry {
                name: "jetstack".into(),
                url: "https://charts.jetstack.io".into(),
            },
        ],
    };
    fake.script().repos.push_ok(entries);
    fake.script().search.push_ok(vec![
        hit(
            "bitnami",
            "nginx",
            "15.4.0",
            "1.25.3",
            "NGINX Open Source is a web server",
        ),
        hit(
            "bitnami",
            "nginx",
            "15.3.0",
            "1.25.2",
            "NGINX Open Source is a web server",
        ),
        hit(
            "bitnami",
            "postgresql",
            "13.2.0",
            "16.1.0",
            "PostgreSQL is an object-relational database",
        ),
        hit(
            "bitnami",
            "redis",
            "18.6.0",
            "7.2.3",
            "Redis is an in-memory data structure store",
        ),
        hit(
            "jetstack",
            "cert-manager",
            "v1.13.2",
            "v1.13.2",
            "A Helm chart for cert-manager",
        ),
        hit(
            "jetstack",
            "trust-manager",
            "v0.7.0",
            "v0.7.0",
            "Distribute trust bundles",
        ),
    ]);
    fake.script().show.push_ok(README.to_owned());
    fake.script().show.push_ok(VALUES.to_owned());
    ChartsService::new(fake, repos.catalog_changes())
}

fn render(scene: Scene) -> anyhow::Result<RgbaImage> {
    let service = service(scene);
    let held: Rc<RefCell<Option<gpui::Entity<HelmChartsView>>>> = Rc::default();
    let hold = held.clone();
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        let tokens = if matches!(scene, Scene::ReadmeLight) {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let deps = ChartsDeps {
            backend: Rc::new(ServiceBackend::new(service)),
            dispatcher: Rc::new(Ignore),
        };
        let view = cx.new(|cx| HelmChartsView::new(deps, window, cx));
        *hold.borrow_mut() = Some(view.clone());
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    cx.run_until_parked();
    if matches!(
        scene,
        Scene::ReadmeDark | Scene::ReadmeLight | Scene::Values
    ) {
        let view = held.borrow().clone().expect("the charts view");
        cx.update_window(window.into(), |_, _, cx| {
            view.update(cx, |view, cx| {
                view.select(0, cx);
                if matches!(scene, Scene::Values) {
                    view.set_tab(Tab::Values, cx);
                }
            });
        })?;
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
        cx.run_until_parked();
    }
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    let shot = cx.capture_screenshot(window.into());
    // The test's handle must not outlive the app, or GPUI reports a leaked entity.
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
        ("charts_list", Scene::List),
        ("charts_readme_dark", Scene::ReadmeDark),
        ("charts_readme_light", Scene::ReadmeLight),
        ("charts_values", Scene::Values),
        ("charts_no_repos", Scene::NoRepos),
    ] {
        if let Err(err) = render(scene).and_then(|image| check(name, image)) {
            eprintln!("{name} screenshot failed: {err:#}");
            return ExitCode::FAILURE;
        }
    }
    println!("helm charts screenshots: ok");
    ExitCode::SUCCESS
}
