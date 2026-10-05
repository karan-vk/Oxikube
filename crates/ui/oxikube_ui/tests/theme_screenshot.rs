//! One sample rendered under two themes (E05-S08): One Dark and One Light, through
//! `oxikube_ui::set_theme`, drawing with the active `oxikube_theme` tokens (including the
//! `oxikube` Kubernetes status colours and the cluster-tab palette).
//!
//! `harness = false` and `--features screenshot` for the reasons given in `screenshot.rs`:
//! `cargo test -p oxikube_ui --features screenshot --test theme_screenshot`.
//!
//! Checks per theme: the PNG has the window's pixel size; the corner pixel is the theme's
//! content background; the frame is not blank; the status colours the sample draws are present
//! in the image; and, when a golden exists for this OS (`tests/goldens/<os>/theme_<name>.png`),
//! the image matches it within tolerance (`OXIKUBE_UPDATE_GOLDENS=1` regenerates). The two
//! images must also differ.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use gpui::{
    AppContext as _, Context, HeadlessAppContext, Hsla, IntoElement, ParentElement as _, Render,
    Rgba, Styled as _, Window, div, px, size,
};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_theme::{ActiveTheme, ThemeRegistry};
use oxikube_ui::{
    ActiveTokens as _,
    button::{Button, ButtonVariants as _},
    layout::{h_flex, v_flex},
};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

const WIDTH: f32 = 520.0;
const HEIGHT: f32 = 260.0;

/// Phases with the `oxikube` colour that paints them.
const PODS: [(&str, &str); 4] = [
    ("api-7d9f", "Running"),
    ("web-5c4b", "Pending"),
    ("db-0", "CrashLoopBackOff"),
    ("migrate-x", "Completed"),
];

struct Sample;

impl Render for Sample {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let theme = ActiveTheme::get(cx);
        let ox = theme.oxikube;
        let phase_color = |phase: &str| match phase {
            "Running" => ox.status_running,
            "Pending" => ox.status_pending,
            "CrashLoopBackOff" => ox.status_failed,
            "Completed" => ox.status_succeeded,
            _ => ox.status_unknown,
        };
        let chip = |color: Hsla| div().size(px(18.)).rounded(px(3.)).bg(color);
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
                    .child(Button::new("delete").danger().label("Delete")),
            )
            .child(h_flex().gap(px(6.)).children(ox.cluster_tabs.map(chip)))
            .child(v_flex().gap(px(4.)).children(PODS.map(|(name, phase)| {
                h_flex()
                    .gap(px(12.))
                    .child(div().w(px(120.)).child(name))
                    .child(chip(phase_color(phase)))
                    .child(div().text_color(phase_color(phase)).child(phase))
            })))
    }
}

fn capture(
    cx: &mut HeadlessAppContext,
    window: gpui::AnyWindowHandle,
) -> anyhow::Result<RgbaImage> {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window)
}

fn rgba_bytes(color: Hsla) -> [u8; 4] {
    let Rgba { r, g, b, a } = Rgba::from(color);
    [r, g, b, a].map(|c| (c * 255.0).round() as u8)
}

fn count_pixels(image: &RgbaImage, color: Hsla) -> usize {
    let wanted = rgba_bytes(color);
    image.pixels().filter(|pixel| pixel.0 == wanted).count()
}

fn render() -> anyhow::Result<Vec<(String, oxikube_theme::ThemeTokens, RgbaImage)>> {
    let text_system = gpui_platform::current_platform(true).text_system();
    let mut cx =
        HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
            gpui_platform::current_headless_renderer()
        });
    let registry = ThemeRegistry::with_bundled();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |_, cx| {
        oxikube_ui::init(cx);
        oxikube_theme::init_with_dir(None, cx);
        cx.new(|_| Sample)
    })?;
    let handle: gpui::AnyWindowHandle = window.into();

    let mut out = Vec::new();
    for name in ["One Dark", "One Light"] {
        let theme = registry
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("no bundled theme {name}"))?;
        cx.update(|cx| {
            oxikube_ui::set_theme(cx, &theme);
            // The sample reads the active theme for the `oxikube` colours.
            cx.set_global(ActiveTheme(theme.clone()));
        });
        out.push((name.to_owned(), (*theme).clone(), capture(&mut cx, handle)?));
    }
    Ok(out)
}

fn run() -> anyhow::Result<()> {
    let shots = render()?;
    let scale = HEADLESS_SCALE_FACTOR;
    let goldens = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    let updating =
        std::env::var_os("OXIKUBE_UPDATE_GOLDENS").is_some_and(|v| v != "0" && !v.is_empty());

    for (name, theme, image) in &shots {
        anyhow::ensure!(
            image.dimensions() == (WIDTH as u32 * scale, HEIGHT as u32 * scale),
            "{name}: unexpected image size {:?}",
            image.dimensions()
        );
        anyhow::ensure!(
            distinct_colors_at_least(image, 8),
            "{name}: frame looks blank"
        );

        let corner = image.get_pixel(2, 2).0;
        let background = rgba_bytes(theme.editor.background);
        anyhow::ensure!(
            corner == background,
            "{name}: corner pixel {corner:02x?} is not the editor background {background:02x?}"
        );
        for (label, color) in [
            ("running", theme.oxikube.status_running),
            ("pending", theme.oxikube.status_pending),
            ("failed", theme.oxikube.status_failed),
            ("succeeded", theme.oxikube.status_succeeded),
            ("cluster tab 1", theme.oxikube.cluster_tabs[0]),
        ] {
            anyhow::ensure!(
                count_pixels(image, color) > 20,
                "{name}: the {label} colour is not drawn"
            );
        }

        let file = format!("theme_{}", name.to_lowercase().replace(' ', "_"));
        let golden = golden_path(&goldens, &file);
        if golden.exists() || updating {
            check_golden(image, &golden, Tolerance::default())?;
            println!("{file} matches {}", golden.display());
        } else {
            println!(
                "no golden for {} yet; structural checks only",
                std::env::consts::OS
            );
        }
    }
    anyhow::ensure!(
        shots[0].2 != shots[1].2,
        "the two themes rendered the same image"
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("theme screenshots: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("theme screenshots failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
