//! The Helm releases page, rendered through `Window::render_to_image` (E16-S05).
//!
//! - `releases_table_dark`, `releases_table_light`: the table with a row of every status tone and
//!   a release selected, its Resources tab open (health, a missing object flagged).
//! - `releases_values_masked`: the Values tab with credential-looking values masked.
//! - `releases_history`: the History tab.
//! - `releases_rollback_preview`: a revision picked for rollback (E16-S06): its chart and the
//!   values it would restore, with the rollback button.
//! - `releases_empty`: no releases in the namespaces being watched.
//!
//! `harness = false` and `--features screenshot`, like the other screenshot tests. Checks the
//! image size, that it is not blank and, when a golden exists for this OS, that it matches within
//! tolerance. Regenerate with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::cell::RefCell;
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use jiff::Timestamp;
use oxikube_domain::command::Command;
use oxikube_domain::helm::{ChartRef, HelmCapabilities, HelmVersion, ReleaseStatus, Revision};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_helm_ui::HelmGateState;
use oxikube_helm_ui::releases::test_support::{StaticBackend, cluster};
use oxikube_helm_ui::releases::{HelmReleasesView, ObjectRow, ObjectState, ReleasesDeps, Tab};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_workspace::CommandDispatcher;

const WIDTH: f32 = 1200.0;
const HEIGHT: f32 = 720.0;

#[derive(Clone, Copy)]
enum Scene {
    TableDark,
    TableLight,
    ValuesMasked,
    History,
    Rollback,
    Empty,
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

fn object(kind: &str, name: &str, state: ObjectState) -> ObjectRow {
    ObjectRow {
        reference: ResourceRef::namespaced(cluster(), Gvk::new("", "v1", kind), "team-0", name),
        source: None,
        state,
    }
}

fn backend(scene: Scene) -> StaticBackend {
    if matches!(scene, Scene::Empty) {
        return StaticBackend::new(0);
    }
    StaticBackend::new(24)
        .with_objects(vec![
            object("Deployment", "nginx-0", ObjectState::Healthy),
            object("Pod", "nginx-0-7d9f-abcde", ObjectState::Healthy),
            object("Pod", "nginx-0-7d9f-fghij", ObjectState::Unhealthy),
            object("Service", "nginx-0", ObjectState::Present),
            object("ConfigMap", "nginx-0-config", ObjectState::Present),
            object("Ingress", "nginx-0", ObjectState::Missing),
        ])
        .with_values(
            "replicaCount: 2\nimage:\n  repository: nginx\n  tag: \"1.25\"\nauth:\n  adminUser: admin\n  \
             adminPassword: ********\n  apiToken: ********\n",
            2,
        )
        .with_history(
            (1..=4)
                .rev()
                .map(|n| Revision {
                    revision: n,
                    status: if n == 4 {
                        ReleaseStatus::Deployed
                    } else {
                        ReleaseStatus::Superseded
                    },
                    chart: ChartRef::new("nginx", format!("1.{n}.0")),
                    app_version: Some(format!("1.2{n}")),
                    updated: Timestamp::from_second(1_791_806_400 - i64::from(n) * 86_400)
                        .expect("in range"),
                    description: if n == 1 {
                        "Install complete".into()
                    } else {
                        "Upgrade complete".into()
                    },
                })
                .collect(),
        )
}

fn render(scene: Scene) -> anyhow::Result<RgbaImage> {
    let held: Rc<RefCell<Option<gpui::Entity<HelmReleasesView>>>> = Rc::default();
    let hold = held.clone();
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        let tokens = if matches!(scene, Scene::TableLight) {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let gate = HelmGateState::install(cx);
        if matches!(scene, Scene::Rollback) {
            // A `helm` binary: the History tab offers rollback.
            gate.update(cx, |gate, cx| {
                gate.set_native_read(true, cx);
                gate.set_cli_capabilities(HelmCapabilities::full(HelmVersion::new(3, 14, 2)), cx);
            });
        }
        let deps = ReleasesDeps {
            backend: Rc::new(backend(scene)),
            dispatcher: Rc::new(Ignore),
            gate,
        };
        let view = cx.new(|cx| HelmReleasesView::new(deps, window, cx));
        *hold.borrow_mut() = Some(view.clone());
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    cx.run_until_parked();
    if !matches!(scene, Scene::Empty) {
        let view = held.borrow().clone().expect("the releases view");
        cx.update_window(window.into(), |_, _, cx| {
            view.update(cx, |view, cx| {
                // The Rollback scene picks a release whose latest revision is 4, as its history.
                view.select_row(
                    if matches!(scene, Scene::Rollback) {
                        3
                    } else {
                        0
                    },
                    cx,
                );
                match scene {
                    Scene::ValuesMasked => view.set_tab(Tab::Values, cx),
                    Scene::History | Scene::Rollback => view.set_tab(Tab::History, cx),
                    _ => {}
                }
            });
        })?;
        cx.run_until_parked();
        if matches!(scene, Scene::Rollback) {
            cx.update_window(window.into(), |_, _, cx| {
                view.update(cx, |view, cx| view.select_revision(2, cx));
            })?;
            cx.run_until_parked();
        }
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
        ("releases_table_dark", Scene::TableDark),
        ("releases_table_light", Scene::TableLight),
        ("releases_values_masked", Scene::ValuesMasked),
        ("releases_history", Scene::History),
        ("releases_rollback_preview", Scene::Rollback),
        ("releases_empty", Scene::Empty),
    ] {
        if let Err(err) = render(scene).and_then(|image| check(name, image)) {
            eprintln!("{name} screenshot failed: {err:#}");
            return ExitCode::FAILURE;
        }
    }
    println!("helm releases screenshots: ok");
    ExitCode::SUCCESS
}
