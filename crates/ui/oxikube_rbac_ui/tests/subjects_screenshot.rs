//! The RBAC subject pages, rendered through `Window::render_to_image` (E17-S07), over the recorded
//! RBAC fixtures of a kind cluster.
//!
//! - `subjects_users_dark`: the Users page, a row selected.
//! - `subjects_groups_dark`: the Groups page with its note that the API lists no members.
//! - `subjects_matrix_dark`, `subjects_matrix_light`: a user's matrix in the namespace it is bound
//!   in, the `pods` row selected and the bindings behind its GET cell listed.
//! - `subjects_matrix_partial_dark`: a group's cluster-wide matrix, with `~` for the ConfigMaps
//!   limited to named objects and the URL rows.
//! - `subjects_unknown_dark`: a name no binding names, with the hint.
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
use oxikube_app::rbac_subjects::PolicyHandle;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_rbac_ui::subjects::{StaticBackend, SubjectsDeps, SubjectsView};
use oxikube_testkit::fixtures::rbac;
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_workspace::CommandDispatcher;

const WIDTH: f32 = 1200.0;
const HEIGHT: f32 = 720.0;

#[derive(Clone, Copy)]
enum Scene {
    UsersDark,
    GroupsDark,
    MatrixDark,
    MatrixLight,
    MatrixPartialDark,
    UnknownDark,
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

fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("kind"))
}

fn render(scene: Scene) -> anyhow::Result<RgbaImage> {
    let held: Rc<RefCell<Option<gpui::Entity<SubjectsView>>>> = Rc::default();
    let hold = held.clone();
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        let tokens = if matches!(scene, Scene::MatrixLight) {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let deps = SubjectsDeps {
            backend: Rc::new(StaticBackend::new(
                cluster(),
                PolicyHandle::fixed(&rbac::cluster()),
            )),
            dispatcher: Rc::new(Ignore),
        };
        let view = cx.new(|cx| SubjectsView::new(deps, None, window, cx));
        *hold.borrow_mut() = Some(view.clone());
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    cx.run_until_parked();
    let view = held.borrow().clone().expect("the subjects view");
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |view, cx| match scene {
            Scene::UsersDark => view.show_users(window, cx),
            Scene::GroupsDark => view.show_groups(window, cx),
            Scene::MatrixDark | Scene::MatrixLight => view.show_subject("fx-viewer", window, cx),
            Scene::MatrixPartialDark => view.show_subject("fx-team", window, cx),
            Scene::UnknownDark => view.show_subject("nobody-at-all", window, cx),
        });
    })?;
    // The reads run on the background executor: let them land, then select.
    cx.run_until_parked();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, _, cx| {
        view.update(cx, |view, cx| match scene {
            Scene::UsersDark => view.select_row(3, cx),
            Scene::MatrixDark | Scene::MatrixLight => {
                let pods = view.grid().read(cx, |d| {
                    (0..d.len()).find(|&row| {
                        d.line(row)
                            .is_some_and(|l| l.api_group.is_empty() && l.resource == "pods")
                    })
                });
                if let Some(row) = pods {
                    view.select_row(row, cx);
                }
            }
            Scene::MatrixPartialDark => view.select_row(1, cx),
            _ => {}
        });
    })?;
    cx.run_until_parked();
    cx.run_until_parked();
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
        ("subjects_users_dark", Scene::UsersDark),
        ("subjects_groups_dark", Scene::GroupsDark),
        ("subjects_matrix_dark", Scene::MatrixDark),
        ("subjects_matrix_light", Scene::MatrixLight),
        ("subjects_matrix_partial_dark", Scene::MatrixPartialDark),
        ("subjects_unknown_dark", Scene::UnknownDark),
    ] {
        if let Err(err) = render(scene).and_then(|image| check(name, image)) {
            eprintln!("{name} screenshot failed: {err:#}");
            return ExitCode::FAILURE;
        }
    }
    println!("rbac subjects screenshots: ok");
    ExitCode::SUCCESS
}
