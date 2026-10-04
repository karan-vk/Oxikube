//! Token sampler screenshot (E05-S02): buttons, icons, an input and a table header rendered with
//! the default tokens through `Window::render_to_image`.
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread, which libtest worker threads are not. Needs a GPU device (Metal, or Vulkan such as Mesa
//! lavapipe on Linux), so it only builds with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_ui --features screenshot --test screenshot`.
//!
//! Checks, in order: the PNG has the window's pixel size; the background is the dark `background`
//! token; the frame is not blank; and, when a golden exists for this OS
//! (`tests/goldens/<os>/token_sampler.png`), the image matches it within tolerance. Regenerate with
//! `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use gpui::{
    App, AppContext as _, Context, Entity, HeadlessAppContext, IntoElement, ParentElement as _,
    Render, Styled as _, Window, div, px, size,
};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_ui::{
    ActiveTokens as _, Icon, IconName,
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    layout::{h_flex, v_flex},
    table::{Table, TableColumn, TableDelegate, TableHandle},
};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

const WIDTH: f32 = 640.0;
const HEIGHT: f32 = 400.0;

struct Pods;

impl TableDelegate for Pods {
    fn columns_count(&self, _: &App) -> usize {
        3
    }
    fn rows_count(&self, _: &App) -> usize {
        4
    }
    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        match col_ix {
            0 => TableColumn::new("name", "Name").width(px(300.)).sortable(),
            1 => TableColumn::new("status", "Status").width(px(160.)),
            _ => TableColumn::new("age", "Age").width(px(120.)).right(),
        }
    }
    fn render_td(
        &mut self,
        row: usize,
        col: usize,
        _: &mut Window,
        _: &mut App,
    ) -> impl IntoElement {
        const NAMES: [&str; 4] = ["api-7d9f", "web-5c4b", "db-0", "cache-1"];
        const STATUS: [&str; 4] = ["Running", "Running", "Pending", "CrashLoopBackOff"];
        match col {
            0 => NAMES[row].to_string(),
            1 => STATUS[row].to_string(),
            _ => format!("{}d", row + 1),
        }
    }
}

struct Sampler {
    input: Entity<InputState>,
    table: TableHandle<Pods>,
}

impl Sampler {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        input.update(cx, |input, cx| {
            input.set_value("kubectl get pods", window, cx)
        });
        Self {
            input,
            table: TableHandle::new(Pods, window, cx),
        }
    }
}

impl Render for Sampler {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let swatch = |color| div().size(px(24.)).rounded(px(4.)).bg(color);
        v_flex()
            .size_full()
            .p(px(16.))
            .gap(px(12.))
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(Button::new("apply").primary().label("Apply"))
                    .child(Button::new("cancel").label("Cancel"))
                    .child(Button::new("delete").danger().label("Delete"))
                    .child(
                        Icon::new(IconName::Box)
                            .size(px(18.))
                            .color(colors.text_muted),
                    )
                    .child(
                        Icon::new(IconName::Layers)
                            .size(px(18.))
                            .color(colors.accent),
                    ),
            )
            .child(Input::new(&self.input))
            .child(
                h_flex()
                    .gap(px(6.))
                    .child(swatch(colors.accent))
                    .child(swatch(colors.success))
                    .child(swatch(colors.warning))
                    .child(swatch(colors.error))
                    .child(swatch(colors.info)),
            )
            .child(
                div()
                    .h(px(180.))
                    .child(Table::new(&self.table).stripe(true)),
            )
    }
}

fn render() -> anyhow::Result<RgbaImage> {
    let text_system = gpui_platform::current_platform(true).text_system();
    let mut cx =
        HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
            gpui_platform::current_headless_renderer()
        });
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.new(|cx| Sampler::new(window, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

fn run() -> anyhow::Result<()> {
    let image = render()?;
    let scale = HEADLESS_SCALE_FACTOR;
    anyhow::ensure!(
        image.dimensions() == (WIDTH as u32 * scale, HEIGHT as u32 * scale),
        "unexpected image size {:?}",
        image.dimensions()
    );
    anyhow::ensure!(distinct_colors_at_least(&image, 8), "frame looks blank");

    // The window background is the dark `background` token (0x1e2127) at the corner.
    let corner = image.get_pixel(2, 2).0;
    anyhow::ensure!(
        corner == [0x1e, 0x21, 0x27, 0xff],
        "corner pixel {corner:02x?} is not the background token"
    );

    let goldens = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    let golden = golden_path(&goldens, "token_sampler");
    let updating =
        std::env::var_os("OXIKUBE_UPDATE_GOLDENS").is_some_and(|v| v != "0" && !v.is_empty());
    if golden.exists() || updating {
        check_golden(&image, &golden, Tolerance::default())?;
        println!("token_sampler matches {}", golden.display());
    } else {
        println!(
            "no golden for {} yet; structural checks only",
            std::env::consts::OS
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("token sampler screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("token sampler screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
