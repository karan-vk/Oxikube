//! The `scroll-10k` scenario (alias `table-scroll-10k`, E07-S09): the generic resource table
//! scrolling 10 000 pods while the feed churns (docs/PERFORMANCE.md "Resource table").
//!
//! It runs the real pieces behind a cluster tab's pods table on testkit fakes: a
//! `ClusterSessionManager` connected through the fake connector, the app's `ResourceStores` on
//! [`store_runtime`] (the probe that feeds `--perf`'s feed counter included) and the
//! [`ResourceTable`] view, in a headless window wrapped in the `--perf` frame hook. The pods come
//! from the fake feed generator ([`fixture`]): a relist of [`PODS`] pods, then one watch batch per
//! frame (see [`fixture::churn`] for the mix and how it compares with the load-pods churn).
//!
//! 1. **Warm**: a subscription lists the feed into the store before the table exists, as when a
//!    view of the kind is already open (the "after feed warm" of the budget).
//! 2. **First rows** (`first_rows_ms`): from creating the table to the end of the first frame that
//!    shows every pod (seeding the table's subscription from the warm cache off the UI thread, the
//!    snapshot, the coalesced notify and the draw; the test clock's notify delay costs no wall
//!    time). Budget: < 1 s (`cargo xtask perf` checks it).
//! 3. **Scroll under churn**: [`FRAMES`] scripted frames; before each, the feed delivers a batch
//!    and the coalesced notify lands (which draws the window, as GPUI's next frame would), then the
//!    frame scrolls [`STEP`] rows and draws. Both draws go through the frame hook, so `frame_ms`
//!    holds two frames per scripted frame, each the full cost of a frame with new rows on screen;
//!    `draw_ms` times the scroll draws from outside.
//!
//! The sample fails (exit 1) unless every batch was counted as feed deltas and no frame absorbed
//! more than one coalesced notify (`max_notifies_per_frame` ≤ 1: the notify path is coalesced to
//! frame cadence whatever the event rate).

use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result, bail, ensure};
use gpui::{
    AnyView, AnyWindowHandle, App, AppContext as _, Context, Entity, HeadlessAppContext,
    IntoElement, Render, Window,
};
use oxikube_app::CoreColumns;
use oxikube_app::store::{ResourceStores, StoreQuery};
use oxikube_ports::ClockPort;
use oxikube_resources_ui::table::{ResourceTable, ResourceTableDeps, store_runtime};
use oxikube_runtime::perf::harness;
use oxikube_runtime::perf::{PerfRoot, Recorder, ScenarioSample, Summary, round_ms};
use oxikube_testkit::{FakeClockPort, FakeStatePort, headless};

use super::WINDOW_SIZE;

mod fixture;

pub use fixture::{FRAME, PODS};

/// Scripted frames: one second at 120 Hz, 240 measured frames. Kept short for the nightly's Linux
/// runner, whose software renderer (lavapipe) draws these frames far slower than a GPU (#509).
pub const FRAMES: usize = 120;
/// Overrides [`FRAMES`] (at most [`FRAMES`]): the smoke test runs a debug build with a few frames.
pub const FRAMES_ENV: &str = "OXIKUBE_PERF_SCROLL_FRAMES";

/// The frames to script: [`FRAMES`], or fewer from [`FRAMES_ENV`].
fn frames() -> usize {
    std::env::var(FRAMES_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(FRAMES, |n: usize| n.clamp(1, FRAMES))
}

/// Rows scrolled per frame: a fast trackpad fling at 120 Hz.
pub const STEP: usize = 3;
/// Upper bound on the executor turns the warm-up and the first rows may take.
const MAX_TURNS: usize = 1_000;

/// The scenario's name (`table-scroll-10k` is accepted for it).
pub const NAME: &str = "scroll-10k";

/// The scenario's own metric.
pub const FIRST_ROWS_MS: &str = "first_rows_ms";

/// One sample. `probe` puts the `--perf` frame hook in the window.
pub fn run(probe: bool) -> Result<ScenarioSample> {
    let recorder = Arc::new(Recorder::new());
    // The process-wide recorder: the stores' feed probe and `notify_coalesced` report to it.
    oxikube_runtime::perf::install(recorder.clone());
    let fixture = fixture::Fixture::connected()?;

    let mut cx = headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets));
    let store_clock: Arc<dyn ClockPort> = Arc::new(FakeClockPort::default());
    let stores = cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
        Arc::new(ResourceStores::new(store_runtime(store_clock, cx)))
    });

    // 1. Warm the feed.
    let session = fixture.session()?;
    let store = stores
        .for_session(&session)
        .context("the store of the connected session")?;
    let kind = fixture::pods_kind();
    let warm = store.subscribe(StoreQuery::new(
        kind.gvk.clone(),
        session.watch_scope(kind.scope()),
    ));
    turn_until(&mut cx, "the feed to list", |_| {
        store
            .feeds()
            .iter()
            .any(|f| f.objects == PODS && f.state.is_ready())
    })?;

    // 2. First rows.
    let deps = ResourceTableDeps {
        sessions: fixture.sessions.clone(),
        stores,
        columns: Arc::new(CoreColumns::new()),
        state: Arc::new(FakeStatePort::new()),
        dispatcher: Rc::new(fixture::Ignore),
        actions: None,
    };
    let hook = probe.then(|| recorder.clone());
    let started = Instant::now();
    let mut table: Option<Entity<ResourceTable>> = None;
    let window: AnyWindowHandle = cx
        .open_window(WINDOW_SIZE, |window, cx| {
            let view =
                cx.new(|cx| ResourceTable::new(fixture.cluster.clone(), kind, deps, window, cx));
            table = Some(view.clone());
            let content: AnyView = match hook {
                Some(recorder) => cx.new(|_| PerfRoot::new(view, recorder)).into(),
                None => view.into(),
            };
            cx.new(|_| Root(content))
        })?
        .into();
    let table = table.context("the table view")?;
    turn_until(&mut cx, "the table to show every pod", |cx| {
        rows(&table, cx) == PODS
    })?;
    harness::draw_frame(&mut cx, window, |_, _| {})?;
    let first_rows = started.elapsed();
    drop(warm);

    // 3. Scroll under churn. The driver parks after each draw: that is where the next batch is
    // delivered and its coalesced notify lands (drawing the window, as GPUI's next frame would).
    let mut reader = recorder.reader();
    reader.drain(&recorder);
    let feed_clock = fixture.feed_clock();
    let scroll = table.clone();
    let run = harness::run_frames(
        &mut cx,
        window,
        frames(),
        &recorder,
        &mut reader,
        |cx| {
            feed_clock.advance(FRAME);
            cx.run_until_parked();
            cx.advance_clock(FRAME);
            cx.run_until_parked();
        },
        |frame, _, cx| {
            let handle = scroll.read(cx).table().clone();
            handle.scroll_to_row((frame * STEP) % PODS, cx);
        },
    )?;

    ensure!(
        run.counters.feed_deltas > 0,
        "no feed delta was counted: the stores' probe is not wired to --perf"
    );
    if run.counters.max_notifies_per_frame > 1 {
        bail!(
            "{} coalesced notifies landed between two frames: the table's notify path is not \
             coalesced to frame cadence",
            run.counters.max_notifies_per_frame
        );
    }
    let first_rows_ms = round_ms(first_rows.as_secs_f64() * 1000.0);
    eprintln!(
        "oxikube scroll-10k: first rows after {first_rows_ms} ms; {} frames, {} feed deltas, {} \
         notifies (at most {} per frame)",
        run.counters.frames,
        run.counters.feed_deltas,
        run.counters.notifies,
        run.counters.max_notifies_per_frame
    );
    Ok(run.into_sample(
        NAME,
        [(FIRST_ROWS_MS.to_owned(), Summary::single(first_rows_ms))],
    ))
}

/// The window's root: the table, behind the frame hook when probing.
struct Root(AnyView);

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0.clone()
    }
}

/// The table's row count.
fn rows(table: &Entity<ResourceTable>, cx: &App) -> usize {
    table.read(cx).read_rows(cx, |d| d.rows().len())
}

/// Runs the executor, advancing the test clock a frame at a time (so coalesced notifies land),
/// until `done`.
fn turn_until(
    cx: &mut HeadlessAppContext,
    what: &str,
    mut done: impl FnMut(&App) -> bool,
) -> Result<()> {
    for _ in 0..MAX_TURNS {
        cx.run_until_parked();
        if cx.update(|cx| done(cx)) {
            return Ok(());
        }
        cx.advance_clock(FRAME);
    }
    bail!("gave up waiting for {what}")
}
