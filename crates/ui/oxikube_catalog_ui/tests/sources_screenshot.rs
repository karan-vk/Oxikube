//! Kubeconfig sources screenshots, rendered through `Window::render_to_image` (E06-S05).
//!
//! - `sources_populated`: the default entry, a file, a pasted kubeconfig stored by Oxikube, a
//!   folder with one skipped file, a file that is not a valid kubeconfig (error inline) and one
//!   that does not exist.
//! - `sources_empty`: no sources, so the explainer.
//!
//! `harness = false` and `--features screenshot`, like the other screenshot tests of this crate
//! (see `catalog_screenshot.rs` for why). Checks the image size, that it is not blank and, when a
//! golden exists for this OS, that it matches within tolerance. Regenerate with
//! `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::sources::SourceRow;
use oxikube_catalog_ui::sources::test_support::{ScriptedBackend, broken, found, row};
use oxikube_catalog_ui::{SourcesDeps, SourcesView};
use oxikube_ports::{SourceState, UserSource};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};

const WIDTH: f32 = 1000.0;
const HEIGHT: f32 = 480.0;

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn populated() -> Vec<SourceRow> {
    vec![
        found(UserSource::default_source(), 3),
        found(UserSource::file("/Users/me/work/prod.yaml"), 2),
        found(UserSource::file("/config/kubeconfigs/staging.yaml"), 1),
        row(
            UserSource::dir("/Users/me/clusters"),
            Some(SourceState::Found),
            4,
            Some("1 of 5 files skipped: old.yaml: not a valid kubeconfig"),
        ),
        broken(
            UserSource::file("/Users/me/work/broken.yaml"),
            SourceState::Invalid,
            "Not a valid kubeconfig",
        ),
        broken(
            UserSource::file("/Users/me/work/gone.yaml"),
            SourceState::Missing,
            "File not found",
        ),
    ]
}

fn render(rows: Vec<SourceRow>) -> anyhow::Result<RgbaImage> {
    let backend = ScriptedBackend::new(rows);
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let deps = SourcesDeps {
            backend: Rc::new(backend),
            workspace: None,
        };
        let view = cx.new(|cx| SourcesView::new(deps, cx));
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
    check("sources_populated", render(populated())?)?;
    check("sources_empty", render(Vec::new())?)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("sources screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("sources screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
