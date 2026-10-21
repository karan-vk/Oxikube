//! The Helm repositories page, rendered through `Window::render_to_image` (E16-S07).
//!
//! - `repos_list`: three repositories after "Update all": one fresh, one failed, one untouched.
//! - `repos_add_form`: the add form opened from a quick-add suggestion.
//! - `repos_no_helm`: no `helm` binary: the guidance and nothing to change.
//!
//! `harness = false` and `--features screenshot`, like the other screenshot tests (the platform
//! text system can only be created on the process main thread). Checks the image size, that it is
//! not blank and, when a golden exists for this OS, that it matches within tolerance. Regenerate
//! with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::HelmReposService;
use oxikube_domain::OxiError;
use oxikube_domain::command::Command;
use oxikube_domain::helm::{HelmCapabilities, RepoEntry};
use oxikube_helm_ui::repos::{HelmReposView, ReposDeps, ServiceBackend};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_testkit::{FakeClockPort, FakeHelmPort};
use oxikube_workspace::CommandDispatcher;

const WIDTH: f32 = 1000.0;
const HEIGHT: f32 = 480.0;

#[derive(Clone, Copy)]
enum Scene {
    List,
    AddForm,
    NoHelm,
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

fn entry(name: &str, url: &str) -> RepoEntry {
    RepoEntry {
        name: name.into(),
        url: url.into(),
    }
}

/// A service whose state the scene sets up before the window exists.
fn service(scene: Scene) -> HelmReposService {
    let fake = Arc::new(match scene {
        Scene::NoHelm => FakeHelmPort::with_capabilities(HelmCapabilities::native_only()),
        _ => FakeHelmPort::new(),
    });
    let service = HelmReposService::new(
        fake.clone(),
        fake.clone(),
        Arc::new(FakeClockPort::default()),
    );
    let repos = match scene {
        Scene::List => vec![
            entry("jetstack", "https://charts.jetstack.io"),
            entry("private", "https://charts.example.com/stable"),
            entry("podinfo", "https://stefanprodan.github.io/podinfo"),
        ],
        Scene::AddForm => vec![entry("jetstack", "https://charts.jetstack.io")],
        Scene::NoHelm => Vec::new(),
    };
    // The page reads the list once more when it opens.
    fake.script().repos.push_ok(repos.clone());
    fake.script().repos.push_ok(repos);
    futures::executor::block_on(async {
        service.refresh().await.ok();
        if matches!(scene, Scene::List) {
            fake.script().repo_update.push_ok(());
            fake.script().repo_update.push_err(OxiError::internal(
                "helm failed (exit code Some(1)): 401 Unauthorized",
            ));
            // The third repository is left alone: update only the first two.
            service
                .update(&["jetstack".into(), "private".into()])
                .await
                .ok();
        }
    });
    service
}

fn render(scene: Scene) -> anyhow::Result<RgbaImage> {
    let service = service(scene);
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let deps = ReposDeps {
            backend: Rc::new(ServiceBackend::new(service)),
            dispatcher: Rc::new(Ignore),
        };
        let view = cx.new(|cx| {
            let mut view = HelmReposView::new(deps, window, cx);
            if matches!(scene, Scene::AddForm) {
                view.use_suggestion(0, window, cx);
            }
            view
        });
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
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
        ("repos_list", Scene::List),
        ("repos_add_form", Scene::AddForm),
        ("repos_no_helm", Scene::NoHelm),
    ] {
        if let Err(err) = render(scene).and_then(|image| check(name, image)) {
            eprintln!("{name} screenshot failed: {err:#}");
            return ExitCode::FAILURE;
        }
    }
    println!("helm repositories screenshots: ok");
    ExitCode::SUCCESS
}
