//! The settings page, rendered through `Window::render_to_image` (E21-S04).
//!
//! - `settings_page_dark` / `settings_page_light`: the General section, in both themes.
//! - `settings_page_search`: the results of a search, the title match highlighted.
//! - `settings_page_user_badge`: the Terminal section with a value from the user's file (the
//!   `User` badge next to `Default` ones).
//!
//! Over the fixed schema of `page::test_support` and a settings store installed from a temporary
//! directory. `harness = false` and `--features screenshot`, like the other screenshot tests (see
//! `oxikube_catalog_ui/tests/catalog_screenshot.rs` for why). Checks the image size, that it is
//! not blank and, when a golden exists for this OS, that it matches within tolerance. Regenerate
//! with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a screenshot test reports on stdout and stderr"
)]

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_settings_ui::page::SettingsPage;
use oxikube_settings_ui::page::test_support::{fixture_model, install_settings};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};

const WIDTH: f32 = 1000.0;
const HEIGHT: f32 = 640.0;

const USER_SETTINGS: &str = r#"{ "terminal": { "font_size": 15 }, "ui_scale": 1.25 }"#;

/// What the page shows when it is captured.
enum Scene {
    /// The first section.
    General,
    /// The results of a query.
    Search(&'static str),
    /// The section with this key.
    Section(&'static str),
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn render(scene: Scene, dark: bool) -> anyhow::Result<RgbaImage> {
    let config = tempfile::tempdir()?;
    let mut cx = headless();
    let mut page = None;
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        let tokens = if dark {
            oxikube_ui::Tokens::dark()
        } else {
            oxikube_ui::Tokens::light()
        };
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        install_settings(config.path(), USER_SETTINGS, cx);
        let view = cx.new(|cx| SettingsPage::with_model(fixture_model(), window, cx));
        page = Some(view.clone());
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    let page = page.ok_or_else(|| anyhow::anyhow!("the page was not built"))?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| match scene {
            Scene::General => {}
            Scene::Search(query) => {
                // The field shows the query, as when it was typed.
                page.query_input()
                    .update(cx, |input, cx| input.set_value(query, window, cx));
                page.set_query(query, cx);
            }
            Scene::Section(key) => {
                if let Some(ix) = page.model().section_index(key) {
                    page.select_section(ix, cx);
                }
            }
        });
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
    check("settings_page_dark", render(Scene::General, true)?)?;
    check("settings_page_light", render(Scene::General, false)?)?;
    check(
        "settings_page_search",
        render(Scene::Search("cursor"), true)?,
    )?;
    check(
        "settings_page_user_badge",
        render(Scene::Section("terminal"), true)?,
    )
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("settings page screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("settings page screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
