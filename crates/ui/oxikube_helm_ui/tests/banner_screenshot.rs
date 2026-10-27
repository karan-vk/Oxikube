//! Screenshots of the Helm page's guidance banner (E16-S11): native-only mode with no `helm`
//! binary (the install/upgrade/rollback/uninstall/charts entries gone, "Locate helm" and
//! "Recheck" offered, the release views still listed) and the same with a too-old binary.
//! Rendered through `Window::render_to_image`, so the pixels are the real view's output.
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread. Needs a GPU device, so it only builds with `--features screenshot` and runs in the
//! nightly job: `cargo test -p oxikube_helm_ui --features screenshot --test banner_screenshot`.
//! Regenerate the goldens with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a harness-less test binary reports its results on stdout and stderr"
)]

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_domain::command::Command;
use oxikube_domain::helm::{HelmCapabilities, HelmVersion};
use oxikube_helm_ui::{HelmGateState, HelmPage};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_workspace::CommandDispatcher;

const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 420.0;

struct Nowhere;

impl CommandDispatcher for Nowhere {
    fn dispatch(&self, _: Command, _: &mut gpui::App) {}
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn render(cli: HelmCapabilities) -> anyhow::Result<RgbaImage> {
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |_, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        let state = HelmGateState::install(cx);
        state.update(cx, |state, cx| {
            state.set_native_read(true, cx);
            state.set_cli_capabilities(cli, cx);
        });
        cx.new(|cx| HelmPage::new(state, Rc::new(Nowhere), cx))
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
    let too_old = HelmCapabilities {
        cli_version: Some(HelmVersion::new(3, 8, 1)),
        ..HelmCapabilities::none()
    };
    for (name, cli) in [
        ("helm_banner_missing_dark", HelmCapabilities::none()),
        ("helm_banner_too_old_dark", too_old),
    ] {
        if let Err(err) = render(cli).and_then(|image| check(name, image)) {
            eprintln!("{name} screenshot failed: {err:#}");
            return ExitCode::FAILURE;
        }
    }
    println!("helm banner screenshots: ok");
    ExitCode::SUCCESS
}
