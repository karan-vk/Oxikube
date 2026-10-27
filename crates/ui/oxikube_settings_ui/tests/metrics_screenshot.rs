//! The cluster metrics page, rendered through `Window::render_to_image` (E13-S05).
//!
//! - `metrics_found`: kube-prometheus found, the services detection looked at, the settings.
//! - `metrics_not_found`: nothing found: `metrics unavailable: <reason>` and every service looked
//!   at, "not found".
//! - `metrics_forbidden`: `list services` refused: the error headline and the way out.
//!
//! `harness = false` and `--features screenshot`, like the other screenshot tests (see
//! `oxikube_catalog_ui/tests/catalog_screenshot.rs` for why). Checks the image size, that it is not
//! blank and, when a golden exists for this OS, that it matches within tolerance. Regenerate with
//! `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a screenshot test reports on stdout and stderr"
)]

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::ProviderStatus;
use oxikube_settings_ui::metrics::test_support::{
    RecordingDispatcher, ScriptedBackend, cluster, forbidden, found, manual, not_found,
};
use oxikube_settings_ui::metrics::{MetricsDeps, MetricsSettingsView};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};

const WIDTH: f32 = 1000.0;
const HEIGHT: f32 = 640.0;

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn render(status: ProviderStatus) -> anyhow::Result<RgbaImage> {
    let backend = ScriptedBackend::new(status);
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let deps = MetricsDeps {
            backend: Rc::new(backend),
            dispatcher: Rc::new(RecordingDispatcher::new()),
        };
        let view = cx.new(|cx| MetricsSettingsView::new(cluster(), deps, cx));
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

fn run() -> anyhow::Result<()> {
    check("metrics_found", render(found())?)?;
    check("metrics_manual", render(manual())?)?;
    check("metrics_not_found", render(not_found())?)?;
    check("metrics_forbidden", render(forbidden())?)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("metrics screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("metrics screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
