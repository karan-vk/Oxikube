//! The `helm-releases` scenario (E16-S05): the Helm releases page over a 500-release fixture,
//! drawn by the page's own layout in a headless window built as the app builds its own
//! ([`window_root`]).
//!
//! 0. **List** (`list_ms`): from creating the page to the end of the first frame that shows all
//!    [`RELEASES`] rows (the list read off the UI thread, the table's rows, the coalesced notify
//!    and the draw), then [`REFRESHES`] more lists (what a change to the release records costs).
//!    Budget: < 1 s. (The native reader's own cost, decoding a record's header, is measured by
//!    `oxikube_helm`'s `scale` test: 500 releases in well under a second.)
//! 1. **Repaint** (`frame_ms` / `draw_ms`, the budget's metrics): [`FRAMES`] frames with nothing
//!    changed, the state of a page that stays open.
//! 2. **Scroll** (`scroll_frame_ms` / `scroll_draw_ms`): the table scrolling the 500 rows, a fling
//!    of [`STEP`] rows per frame.
//! 3. **Select** (`select_frame_ms` / `select_draw_ms`): a different release selected on every
//!    frame, so its detail (header, tabs, the Resources tab's skeleton and then its data) opens
//!    anew each time, the worst case of holding the down key.
//!
//! The sample fails when the table does not show all rows, or when the view's text is drawn in a
//! family the machine lacks (#509).

use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result, ensure};
use gpui::{AnyWindowHandle, AppContext as _, Entity};
use oxikube_domain::command::Command;
use oxikube_helm_ui::HelmGateState;
use oxikube_helm_ui::releases::test_support::StaticBackend;
use oxikube_helm_ui::releases::{HelmReleasesView, ReleasesDeps};
use oxikube_runtime::perf::harness::{self, FrameRun};
use oxikube_runtime::perf::{Recorder, ScenarioSample, Summary};
use oxikube_testkit::headless;
use oxikube_workspace::CommandDispatcher;

use super::{FRAMES, WINDOW_SIZE, window_root};

/// The scenario's name.
pub(super) const NAME: &str = "helm-releases";

/// Overrides the frames per phase (a debug build in the smoke test uses a few).
const FRAMES_ENV: &str = "OXIKUBE_PERF_HELM_RELEASES_FRAMES";

/// The fixture's size: the epic's "500 releases".
pub(super) const RELEASES: usize = 500;
/// Lists after the first one.
const REFRESHES: usize = 20;
/// Rows scrolled per frame.
const STEP: usize = 4;
/// Upper bound on the executor turns a list may take.
const MAX_TURNS: usize = 1_000;

fn frames() -> usize {
    std::env::var(FRAMES_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(FRAMES, |n: usize| n.clamp(1, FRAMES))
}

/// Sends nothing anywhere: the scenario presses no buttons.
struct NoBus;

impl CommandDispatcher for NoBus {
    fn dispatch(&self, _: Command, _: &mut gpui::App) {}
}

fn shown(view: &Entity<HelmReleasesView>, cx: &gpui::App) -> usize {
    view.read(cx).table().read(cx, |d| d.len())
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
    let mut page: Option<Entity<HelmReleasesView>> = None;
    let mut text_font = None;
    let started = Instant::now();
    let window_handle: AnyWindowHandle = cx
        .open_window(WINDOW_SIZE, |window, cx| {
            let gate = HelmGateState::install(cx);
            let deps = ReleasesDeps {
                backend: Rc::new(StaticBackend::new(RELEASES)),
                dispatcher: Rc::new(NoBus),
                gate,
            };
            let view = cx.new(|cx| HelmReleasesView::new(deps, window, cx));
            page = Some(view.clone());
            let (root, font) = window_root::mount(view.into(), hook, window, cx);
            text_font = Some(font);
            root
        })?
        .into();
    let page = page.context("the releases page")?;

    // The first list: until every row is on screen.
    let mut turns = 0;
    while cx.update(|cx| shown(&page, cx)) < RELEASES {
        ensure!(
            turns < MAX_TURNS,
            "the 500 releases never reached the table"
        );
        turns += 1;
        cx.run_until_parked();
        harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
    }
    let first_list = started.elapsed();
    // Two more frames: the first lays everything out, the second paints from the caches.
    harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
    harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
    cx.update(|cx| text_font.context("the window root")?.check(cx))?;

    // Lists after a change: each is one read off the UI thread and the table's rows rebuilt.
    let mut refresh_ns = Vec::with_capacity(REFRESHES);
    for _ in 0..REFRESHES {
        let at = Instant::now();
        page.update(&mut cx, |page, cx| page.refresh(cx));
        cx.run_until_parked();
        harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
        refresh_ns.push(u64::try_from(at.elapsed().as_nanos()).unwrap_or(u64::MAX));
        ensure!(
            cx.update(|cx| shown(&page, cx)) == RELEASES,
            "a refresh lost rows"
        );
    }

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

    // 2. Scroll.
    let scroll_page = page.clone();
    let mut scroll = phase(&mut cx, &mut |frame, _, cx| {
        let table = scroll_page.read(cx).table().clone();
        table.scroll_to_row((frame * STEP) % RELEASES, cx);
    })?;

    // 3. Select: a new release (and so a new detail) on every frame.
    let select_page = page.clone();
    let mut select = phase(&mut cx, &mut |frame, _, cx| {
        select_page.update(cx, |page, cx| page.select_row((frame * 7) % RELEASES, cx));
    })?;
    ensure!(
        cx.update(|cx| page.read(cx).selected_release().is_some()),
        "no detail was open"
    );

    let mut extra: Vec<(String, Summary)> = Vec::new();
    extra.push((
        "first_list_ms".to_owned(),
        Summary::single(first_list.as_secs_f64() * 1000.0),
    ));
    if let Some(summary) = Summary::from_nanos(&mut refresh_ns) {
        extra.push(("list_ms".to_owned(), summary));
    }
    for (prefix, run) in [("scroll_", &mut scroll), ("select_", &mut select)] {
        for (name, samples) in [
            (harness::metric::FRAME_MS, &mut run.frame_ns),
            (harness::metric::DRAW_MS, &mut run.draw_ns),
        ] {
            if let Some(summary) = Summary::from_nanos(samples) {
                extra.push((format!("{prefix}{name}"), summary));
            }
        }
    }
    eprintln!(
        "oxikube helm-releases: {RELEASES} releases, first list {:.1} ms, {} frames per phase",
        first_list.as_secs_f64() * 1000.0,
        main.counters.frames
    );
    Ok(main.into_sample(NAME, extra))
}
