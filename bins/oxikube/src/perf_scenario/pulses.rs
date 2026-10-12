//! The `pulses` scenario (E13-S10): the pulses page with its sixteen tiles (fourteen sparklines
//! of 60 samples, two gauges), drawn by the page's own layout in a headless window built as the
//! app builds its own ([`window_root`]).
//!
//! 1. **Repaint** (`frame_ms` / `draw_ms`, the budget's metrics): [`FRAMES`] frames with nothing
//!    changed, the state of a page that stays open between samples. Geometry is cached, so it is
//!    the paint cost.
//! 2. **Tick** (`tick_frame_ms` / `tick_draw_ms`): every frame gives every tile a new sample, the
//!    worst case (the page samples every 5 s): each sparkline re-decimates and re-tessellates.
//!
//! The sample fails when the view's text is drawn in a family the machine lacks (#509).

use std::sync::Arc;

use anyhow::{Context as _, Result};
use gpui::{AnyWindowHandle, AppContext as _, Entity};
use oxikube_overview_ui::pulses::test_support::{PREVIEW_NOW, PulsesPreview, preview_model};
use oxikube_runtime::perf::harness::{self, FrameRun};
use oxikube_runtime::perf::{Recorder, ScenarioSample, Summary};
use oxikube_testkit::headless;

use super::{FRAMES, WINDOW_SIZE, window_root};

/// The scenario's name.
pub(super) const NAME: &str = "pulses";

/// Overrides the frames per phase (a debug build in the smoke test uses a few).
const FRAMES_ENV: &str = "OXIKUBE_PERF_PULSES_FRAMES";

fn frames() -> usize {
    std::env::var(FRAMES_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(FRAMES, |n: usize| n.clamp(1, FRAMES))
}

/// One sample. `probe` puts the `--perf` frame hook in the window.
pub(super) fn run(probe: bool) -> Result<ScenarioSample> {
    let recorder = Arc::new(Recorder::new());
    oxikube_runtime::perf::install(recorder.clone());
    let frames = frames();

    let mut cx = headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets));
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
    });
    let hook = probe.then(|| recorder.clone());
    let mut page: Option<Entity<PulsesPreview>> = None;
    let mut text_font = None;
    let window_handle: AnyWindowHandle = cx
        .open_window(WINDOW_SIZE, |window, cx| {
            let view = cx.new(|_| PulsesPreview::new(preview_model()));
            page = Some(view.clone());
            let (root, font) = window_root::mount(view.into(), hook, window, cx);
            text_font = Some(font);
            root
        })?
        .into();
    let page = page.context("the pulses page")?;
    // Two frames: the first lays everything out, the second paints from the caches.
    harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
    harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
    cx.update(|cx| text_font.context("the window root")?.check(cx))?;

    let phase = |cx: &mut gpui::HeadlessAppContext,
                 step: &mut dyn FnMut(usize, &mut gpui::Window, &mut gpui::App)|
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

    // 2. Tick: every tile takes a new sample on every frame.
    let tick_page = page.clone();
    let mut tick = phase(&mut cx, &mut |frame, _, cx| {
        tick_page.update(cx, |page, cx| {
            let at = PREVIEW_NOW + 5.0 * (frame + 1) as f64;
            for (i, tile) in page.model_mut().tiles_mut().iter_mut().enumerate() {
                let latest = tile.ring.latest().unwrap_or(1.0);
                let wobble = ((frame + i) % 7) as f64 - 3.0;
                tile.ring.push(at, (latest + wobble).max(0.0));
            }
            cx.notify();
        });
    })?;

    let extra: Vec<(String, Summary)> = [
        (harness::metric::FRAME_MS, &mut tick.frame_ns),
        (harness::metric::DRAW_MS, &mut tick.draw_ns),
    ]
    .into_iter()
    .filter_map(|(name, samples)| Summary::from_nanos(samples).map(|s| (format!("tick_{name}"), s)))
    .collect();
    eprintln!(
        "oxikube pulses: 16 tiles, 14 sparklines of 60 samples, {} frames per phase",
        main.counters.frames
    );
    Ok(main.into_sample(NAME, extra))
}
