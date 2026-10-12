//! The pulses page, rendered through `Window::render_to_image` (E13-S10).
//!
//! - `pulses_busy`: a busy four-node cluster: every kind counted with its breakdown and history,
//!   the ingresses over the watch budget and counted once, the two gauges with their history and
//!   their threshold levels, the pods tile selected.
//! - `pulses_no_metrics`: the same cluster without metrics-server: the gauges say
//!   `metrics unavailable: <reason>` and draw no number.
//!
//! `harness = false` and `--features screenshot`, like the other screenshot tests (see
//! `oxikube_catalog_ui/tests/catalog_screenshot.rs` for why). Checks the image size, that it is not
//! blank and, when a golden exists for this OS, that it matches within tolerance. Regenerate with
//! `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_overview_ui::pulses::test_support::{PulsesPreview, preview_model};
use oxikube_overview_ui::pulses::{ClusterFigure, PulsesModel};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};

const WIDTH: f32 = 1100.0;
const HEIGHT: f32 = 760.0;

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn render(model: PulsesModel) -> anyhow::Result<RgbaImage> {
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let view = cx.new(|_| PulsesPreview::new(model));
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

fn without_metrics() -> PulsesModel {
    let mut model = preview_model();
    for id in ["cluster-cpu", "cluster-memory"] {
        model.set_cluster(
            id,
            ClusterFigure::Unavailable(
                "metrics unavailable: metrics-server is not installed (no metrics.k8s.io API)"
                    .to_owned(),
            ),
            Vec::new(),
        );
    }
    model
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
    check("pulses_busy", render(preview_model())?)?;
    check("pulses_no_metrics", render(without_metrics())?)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("pulses screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("pulses screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
