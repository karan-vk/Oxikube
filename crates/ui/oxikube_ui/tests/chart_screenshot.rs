//! Chart screenshots (E13-S07): line, area, bar, sparkline and the whole gallery, each under One
//! Dark and One Light, through `oxikube_ui::set_theme`.
//!
//! `harness = false` and `--features screenshot` for the reasons given in `screenshot.rs`:
//! `cargo test -p oxikube_ui --features screenshot --test chart_screenshot`.
//!
//! Checks per image: the PNG has the window's pixel size; the corner pixel is the theme's
//! background; the frame is not blank; every series colour the case draws is present in the image;
//! and, when a golden exists for this OS (`tests/goldens/<os>/chart_<case>_<theme>.png`), the image
//! matches it within tolerance (`OXIKUBE_UPDATE_GOLDENS=1` regenerates). The two themes of a case
//! must differ.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use gpui::{
    AnyWindowHandle, AppContext as _, Context, Entity, HeadlessAppContext, Hsla, IntoElement,
    ParentElement as _, Render, Rgba, Styled as _, Window, div, px, size,
};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_theme::ThemeRegistry;
use oxikube_ui::chart::gallery::{ChartGallery, fixtures::wave};
use oxikube_ui::chart::{
    Band, Chart, ChartPalette, Point, Series, SeriesColor, Sparkline, Threshold, TimeAxis, Tone,
    ValueFormat, YRange,
};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Tokens};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

/// 2026-10-12 12:00 UTC; the fixtures end here, so the pictures are stable.
const NOW: f64 = 1_791_806_400.0;

/// One picture: a window size and the view to show.
struct Case {
    name: &'static str,
    size: (f32, f32),
    /// How many series colours the picture must show.
    series: usize,
    build: fn(&mut Window, &mut gpui::App) -> gpui::AnyView,
}

/// The window's root: whichever view a case builds.
struct Shell(gpui::AnyView);

impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
    }
}

/// A chart in a padded, filled box, as a detail panel would show it.
struct Panel(Entity<Chart>);

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p(px(16.))
            .bg(cx.colors().background)
            .child(self.0.clone())
    }
}

fn panel(
    chart: Chart,
    series: Vec<Series>,
    axis: TimeAxis,
    hover: Option<f64>,
    cx: &mut gpui::App,
) -> gpui::AnyView {
    let chart = cx.new(|_| chart);
    chart.update(cx, |chart, cx| {
        chart.set_series(series, axis, cx);
        chart.hover_time(hover, cx);
    });
    cx.new(|_| Panel(chart)).into()
}

fn line(_: &mut Window, cx: &mut gpui::App) -> gpui::AnyView {
    let series = vec![
        wave("nginx", NOW, 240, 15.0, 0.45, 0.2, 40.0, 1),
        wave("sidecar", NOW, 240, 15.0, 0.12, 0.05, 25.0, 2),
        wave("init-db", NOW, 240, 15.0, 0.62, 0.25, 60.0, 3),
    ];
    let hover = series[0].points[150].t;
    panel(
        Chart::line("line")
            .format(ValueFormat::Cores)
            .y_range(YRange::Fixed(0.0, 1.0))
            .thresholds(vec![
                Threshold::new(0.5, "request"),
                Threshold::new(0.9, "limit").tone(Tone::Critical),
            ])
            .bands(vec![Band::new(0.9, 1.0, Tone::Critical)]),
        series,
        TimeAxis::utc(NOW - 3600.0, NOW),
        Some(hover),
        cx,
    )
}

fn area(_: &mut Window, cx: &mut gpui::App) -> gpui::AnyView {
    let gib = 1024.0 * 1024.0 * 1024.0;
    panel(
        Chart::area("area")
            .format(ValueFormat::Bytes)
            .thresholds(vec![Threshold::new(3.0 * gib, "limit").tone(Tone::Warn)]),
        vec![wave(
            "working set",
            NOW,
            5_760,
            15.0,
            1.6 * gib,
            0.5 * gib,
            900.0,
            4,
        )],
        TimeAxis::utc(NOW - 86_400.0, NOW),
        None,
        cx,
    )
}

fn bar(_: &mut Window, cx: &mut gpui::App) -> gpui::AnyView {
    panel(
        Chart::bar("bar").format(ValueFormat::BytesPerSecond),
        vec![
            wave("rx", NOW, 72, 300.0, 4.0e6, 3.0e6, 18.0, 5),
            wave("tx", NOW, 72, 300.0, 1.5e6, 1.0e6, 12.0, 6),
        ],
        TimeAxis::utc(NOW - 6.0 * 3600.0, NOW),
        None,
        cx,
    )
}

struct Sparks(Vec<(Arc<[Point]>, SeriesColor, bool)>);

impl Render for Sparks {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        v_flex()
            .size_full()
            .p(px(16.))
            .gap(px(12.))
            .bg(colors.background)
            .child(
                h_flex()
                    .gap(px(16.))
                    .children(self.0.iter().enumerate().map(|(i, (points, color, fill))| {
                        div().w(px(120.)).child(
                            Sparkline::new(("spark", i), points.clone())
                                .color(*color)
                                .filled(*fill)
                                .last_marker(true)
                                .height(px(32.)),
                        )
                    })),
            )
    }
}

fn sparklines(_: &mut Window, cx: &mut gpui::App) -> gpui::AnyView {
    let spark = |seed, color, fill| {
        (
            wave("s", NOW, 120, 15.0, 40.0, 25.0, 30.0, seed).points,
            color,
            fill,
        )
    };
    cx.new(|_| {
        Sparks(vec![
            spark(11, SeriesColor::Auto, false),
            spark(12, SeriesColor::Tone(Tone::Good), true),
            spark(13, SeriesColor::Tone(Tone::Warn), false),
            spark(14, SeriesColor::Tone(Tone::Critical), true),
        ])
    })
    .into()
}

fn gallery(_: &mut Window, cx: &mut gpui::App) -> gpui::AnyView {
    cx.new(|cx| ChartGallery::new(NOW, cx)).into()
}

const CASES: [Case; 5] = [
    Case {
        name: "line",
        size: (640.0, 300.0),
        series: 3,
        build: line,
    },
    Case {
        name: "area",
        size: (640.0, 280.0),
        series: 1,
        build: area,
    },
    Case {
        name: "bar",
        size: (640.0, 280.0),
        series: 2,
        build: bar,
    },
    Case {
        name: "sparkline",
        size: (640.0, 80.0),
        series: 1,
        build: sparklines,
    },
    Case {
        name: "gallery",
        size: (760.0, 1800.0),
        series: 3,
        build: gallery,
    },
];

fn rgba_bytes(color: Hsla) -> [u8; 4] {
    let Rgba { r, g, b, a } = Rgba::from(color);
    [r, g, b, a].map(|c| (c * 255.0).round() as u8)
}

/// Pixels within `slack` per channel of `color` (lines are anti-aliased, so exact matches are few).
fn count_near(image: &RgbaImage, color: Hsla, slack: u8) -> usize {
    let want = rgba_bytes(color);
    image
        .pixels()
        .filter(|p| {
            p.0.iter()
                .take(3)
                .zip(&want)
                .all(|(a, b)| a.abs_diff(*b) <= slack)
        })
        .count()
}

fn capture(cx: &mut HeadlessAppContext, window: AnyWindowHandle) -> anyhow::Result<RgbaImage> {
    // Two frames: the first lays the charts out, the second paints from their cached geometry.
    for _ in 0..2 {
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))?;
    }
    cx.run_until_parked();
    cx.capture_screenshot(window)
}

fn render() -> anyhow::Result<Vec<(String, &'static Case, oxikube_theme::ThemeTokens, RgbaImage)>> {
    let text_system = gpui_platform::current_platform(true).text_system();
    let mut cx =
        HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
            gpui_platform::current_headless_renderer()
        });
    let registry = ThemeRegistry::with_bundled();
    let mut out = Vec::new();
    for name in ["One Dark", "One Light"] {
        let theme = registry
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("no bundled theme {name}"))?;
        for case in &CASES {
            let (w, h) = case.size;
            let theme_for_window = theme.clone();
            let window = cx.open_window(size(px(w), px(h)), move |window, cx| {
                oxikube_ui::init(cx);
                oxikube_theme::init_with_dir(None, cx);
                oxikube_ui::set_theme(cx, &theme_for_window);
                let view = (case.build)(window, cx);
                cx.new(|_| Shell(view))
            })?;
            let handle: AnyWindowHandle = window.into();
            let image = capture(&mut cx, handle)?;
            out.push((name.to_owned(), case, (*theme).clone(), image));
            cx.update_window(handle, |_, window, _| window.remove_window())?;
        }
    }
    Ok(out)
}

fn run() -> anyhow::Result<()> {
    let shots = render()?;
    let scale = HEADLESS_SCALE_FACTOR;
    let goldens = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    let updating =
        std::env::var_os("OXIKUBE_UPDATE_GOLDENS").is_some_and(|v| v != "0" && !v.is_empty());

    for (theme_name, case, theme, image) in &shots {
        let label = format!("{} / {theme_name}", case.name);
        let (w, h) = case.size;
        anyhow::ensure!(
            image.dimensions() == (w as u32 * scale, h as u32 * scale),
            "{label}: unexpected image size {:?}",
            image.dimensions()
        );
        anyhow::ensure!(
            distinct_colors_at_least(image, 8),
            "{label}: frame looks blank"
        );
        let tokens = Tokens::from(theme);
        let corner = image.get_pixel(2, 2).0;
        let background = rgba_bytes(tokens.colors.background);
        anyhow::ensure!(
            corner == background,
            "{label}: corner {corner:02x?} is not the background {background:02x?}"
        );

        let palette = ChartPalette::from_tokens(&tokens);
        for (i, colour) in palette.series.iter().take(case.series).enumerate() {
            anyhow::ensure!(
                count_near(image, *colour, 10) > 30,
                "{label}: series {i} colour is not drawn"
            );
        }

        let file = format!(
            "chart_{}_{}",
            case.name,
            theme_name.to_lowercase().replace(' ', "_")
        );
        let golden = golden_path(&goldens, &file);
        if golden.exists() || updating {
            check_golden(image, &golden, Tolerance::default())?;
            println!("{file} matches {}", golden.display());
        } else {
            println!(
                "no golden for {} yet ({file}); structural checks only",
                std::env::consts::OS
            );
        }
    }
    for case in &CASES {
        let mut images = shots
            .iter()
            .filter(|(_, c, ..)| c.name == case.name)
            .map(|(.., image)| image);
        let (dark, light) = (images.next(), images.next());
        anyhow::ensure!(
            dark != light,
            "{}: the two themes rendered the same image",
            case.name
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("chart screenshots: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("chart screenshots failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
