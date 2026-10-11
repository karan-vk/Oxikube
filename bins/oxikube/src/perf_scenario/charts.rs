//! The `charts` scenario (E13-S07): the overview's worst case, 12 sparklines and 3 area charts of
//! 5 760 points each (24 h at a 15 s step), drawn by the `oxikube_ui` chart components in a
//! headless window built as the app builds its own ([`window_root`]).
//!
//! 1. **Repaint** (`frame_ms` / `draw_ms`, the budget's metrics): [`FRAMES`] frames with nothing
//!    changed, the state of a view that stays open between metrics ticks. Every frame renders the
//!    sparklines and charts again; the geometry is cached, so it is the paint cost.
//! 2. **Tick** (`tick_frame_ms` / `tick_draw_ms`): every frame hands all 15 new data (the window
//!    slides one 15 s step), the worst case of a poll that lands on every frame; each piece
//!    re-decimates, re-projects and re-tessellates once.
//! 3. **Hover** (`hover_frame_ms` / `hover_draw_ms`): the tooltip of one chart follows a cursor
//!    sweeping its width, one sample per frame.
//!
//! The sample fails when a repaint rebuilt any geometry, when a tick built more than once per
//! piece, or when the view's text is drawn in a family the machine lacks (#509).

use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use gpui::{
    AnyWindowHandle, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, px,
};
use oxikube_runtime::perf::harness::{self, FrameRun};
use oxikube_runtime::perf::{Recorder, ScenarioSample, Summary};
use oxikube_testkit::headless;
use oxikube_ui::ActiveTokens as _;
use oxikube_ui::chart::gallery::fixtures::wave;
use oxikube_ui::chart::{Chart, Point, Series, Sparkline, Threshold, TimeAxis, ValueFormat};
use oxikube_ui::layout::{h_flex, v_flex};

use super::{FRAMES, WINDOW_SIZE, window_root};

/// The scenario's name.
pub(super) const NAME: &str = "charts";

/// Overrides the frames per phase (a debug build in the smoke test uses a few).
const FRAMES_ENV: &str = "OXIKUBE_PERF_CHARTS_FRAMES";

const SPARKLINES: usize = 12;
const CHARTS: usize = 3;
/// 24 h at 15 s.
const POINTS: usize = 5_760;
const SPARK_POINTS: usize = 120;
const STEP: f64 = 15.0;
/// The window's last sample time; fixed so every run draws the same pixels.
const NOW: f64 = 1_791_806_400.0;

fn frames() -> usize {
    std::env::var(FRAMES_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(FRAMES, |n: usize| n.clamp(1, FRAMES))
}

/// The page: a row of sparklines over three area charts, as the overview lays them out.
struct Overview {
    sparks: Vec<Arc<[Point]>>,
    charts: Vec<Entity<Chart>>,
}

impl Render for Overview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .p(px(16.))
            .gap(px(12.))
            .bg(cx.colors().background)
            .child(
                h_flex()
                    .gap(px(12.))
                    .children(self.sparks.iter().enumerate().map(|(i, points)| {
                        div()
                            .w(px(90.))
                            .child(Sparkline::new(("spark", i), points.clone()).filled(i % 2 == 0))
                    })),
            )
            .children(
                self.charts
                    .iter()
                    .map(|chart| div().h(px(190.)).w_full().child(chart.clone())),
            )
    }
}

/// The `count` samples of `base` (which holds `frames` more than that) that end `shift` steps
/// after [`NOW`]: the data window after `shift` ticks.
fn window_of(base: &[Point], shift: usize, count: usize, frames: usize) -> Arc<[Point]> {
    let end = base.len() - frames + shift;
    base[end - count..end].into()
}

/// One sample. `probe` puts the `--perf` frame hook in the window.
pub(super) fn run(probe: bool) -> Result<ScenarioSample> {
    let recorder = Arc::new(Recorder::new());
    oxikube_runtime::perf::install(recorder.clone());
    let frames = frames();

    // The data: long enough that sliding by a step per frame never runs out.
    let long = |seed: u64, count: usize| {
        wave(
            "s",
            NOW + STEP * frames as f64,
            count + frames,
            STEP,
            50.0,
            25.0,
            900.0,
            seed,
        )
        .points
    };
    let area_data: Vec<Arc<[Point]>> = (0..CHARTS).map(|i| long(i as u64 + 1, POINTS)).collect();
    let spark_data: Vec<Arc<[Point]>> = (0..SPARKLINES)
        .map(|i| long(100 + i as u64, SPARK_POINTS))
        .collect();
    let axis = |shift: usize| {
        let end = NOW + STEP * shift as f64;
        TimeAxis::utc(end - 86_400.0, end)
    };

    let mut cx = headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets));
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
    });
    let hook = probe.then(|| recorder.clone());
    let mut page: Option<Entity<Overview>> = None;
    let mut text_font = None;
    let window_handle: AnyWindowHandle = cx
        .open_window(WINDOW_SIZE, |window, cx| {
            let charts: Vec<Entity<Chart>> = (0..CHARTS)
                .map(|i| {
                    let chart = cx.new(|_| {
                        Chart::area(("area", i))
                            .format(ValueFormat::Percent)
                            .thresholds(vec![Threshold::new(90.0, "limit")])
                    });
                    let series = Series::new("usage", window_of(&area_data[i], 0, POINTS, frames));
                    chart.update(cx, |chart, cx| chart.set_series(vec![series], axis(0), cx));
                    chart
                })
                .collect();
            let sparks = spark_data
                .iter()
                .map(|d| window_of(d, 0, SPARK_POINTS, frames))
                .collect();
            let view = cx.new(|_| Overview { sparks, charts });
            page = Some(view.clone());
            let (root, font) = window_root::mount(view.into(), hook, window, cx);
            text_font = Some(font);
            root
        })?
        .into();
    let page = page.context("the overview page")?;
    // Two frames: the first lays everything out, the second paints from the caches.
    harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
    harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
    cx.update(|cx| text_font.context("the window root")?.check(cx))?;
    let builds = |cx: &gpui::App| -> Vec<usize> {
        page.read(cx)
            .charts
            .iter()
            .map(|c| c.read(cx).geometry_builds())
            .collect()
    };
    let warm = cx.update(|cx| builds(cx));
    ensure!(
        warm.iter().all(|n| *n == 1),
        "each chart laid out once: {warm:?}"
    );

    let phase = |cx: &mut gpui::HeadlessAppContext,
                 step: &mut dyn FnMut(usize, &mut Window, &mut gpui::App)|
     -> Result<FrameRun> {
        let mut reader = recorder.reader();
        reader.drain(&recorder);
        harness::run_frames(
            cx,
            window_handle,
            frames,
            &recorder,
            &mut reader,
            |cx| cx.run_until_parked(),
            step,
        )
    };

    // 1. Repaint.
    let main = phase(&mut cx, &mut |_, _, _| {})?;
    let after_repaint = cx.update(|cx| builds(cx));
    ensure!(
        after_repaint == warm,
        "repainting unchanged charts rebuilt their geometry: {warm:?} -> {after_repaint:?}"
    );

    // 2. Tick: every piece gets new data every frame.
    let (tick_page, tick_area, tick_spark) = (page.clone(), area_data.clone(), spark_data.clone());
    let mut tick = phase(&mut cx, &mut |frame, _, cx| {
        let shift = frame + 1;
        tick_page.update(cx, |page, cx| {
            for (slot, data) in page.sparks.iter_mut().zip(&tick_spark) {
                *slot = window_of(data, shift, SPARK_POINTS, frames);
            }
            for (chart, data) in page.charts.iter().zip(&tick_area) {
                let series = Series::new("usage", window_of(data, shift, POINTS, frames));
                chart.update(cx, |chart, cx| {
                    chart.set_series(vec![series], axis(shift), cx)
                });
            }
            cx.notify();
        });
    })?;
    let after_tick = cx.update(|cx| builds(cx));
    ensure!(
        after_tick.iter().all(|n| *n == 1 + frames),
        "a tick must rebuild each chart exactly once: {after_tick:?} after {frames} ticks"
    );

    // 3. Hover: the tooltip of the first chart follows the cursor.
    let hovered = page.read_with(&cx, |page, _| page.charts[0].clone());
    let end = NOW + STEP * frames as f64;
    let mut hover = phase(&mut cx, &mut |frame, _, cx| {
        let t = end - 86_400.0 + 86_400.0 * (frame as f64 + 0.5) / frames as f64;
        hovered.update(cx, |chart, cx| chart.hover_time(Some(t), cx));
    })?;
    hovered.update(&mut cx, |chart, cx| chart.hover_time(None, cx));

    let extra: Vec<(String, Summary)> = [("tick_", &mut tick), ("hover_", &mut hover)]
        .into_iter()
        .flat_map(|(prefix, run)| {
            [
                (harness::metric::FRAME_MS, &mut run.frame_ns),
                (harness::metric::DRAW_MS, &mut run.draw_ns),
            ]
            .into_iter()
            .filter_map(move |(name, samples)| {
                Summary::from_nanos(samples).map(|s| (format!("{prefix}{name}"), s))
            })
            .collect::<Vec<_>>()
        })
        .collect();
    eprintln!(
        "oxikube charts: {SPARKLINES} sparklines and {CHARTS} area charts of {POINTS} points, {} \
         frames per phase",
        main.counters.frames
    );
    Ok(main.into_sample(NAME, extra))
}
