//! The `logs-stream` scenario (E08-S02): the log view streaming [`LINES_PER_S`] lines a second
//! (docs/PERFORMANCE.md "Log viewer").
//!
//! It runs the real pieces behind a log tab on testkit fakes: a `ClusterSessionManager` connected
//! through the fake connector, the app's `LogService` (default config: batches of 2 048 lines or
//! one 32 ms tick, the 50 000-line ring buffer) and the [`LogView`], in a headless window wrapped
//! in the `--perf` frame hook. The pod's log ([`fixture`]) is a 1 000-line tail, then
//! [`LINES_PER_S`] lines a second replayed on the log port's clock, one [`FRAME`] of it per
//! scripted frame.
//!
//! 1. **Tail**: the view opens (reading the pod for its default container, then the stream) and
//!    the clocks run a frame at a time until the tail is on screen.
//! 2. **Six modes**, [`FRAMES`] scripted frames each, in one process and one stream: wrap off
//!    and following (the budget's mode: `frame_ms` / `draw_ms`), then autoscroll paused
//!    (`paused_*`), wrapped and following (`wrap_*`), wrapped and paused (`wrap_paused_*`), JSON
//!    mode off (`raw_*`, for comparison), JSON mode with the debug and plain-text chips off
//!    (`json_filtered_*`), and, with a search (E08-S03, [`PATTERN`], wrap off and following),
//!    highlighting (`search_*`) and filtering (`filter_*`), where every delta also tests its new
//!    lines against the pattern, and the filtered rows are the matches only. The pod's log is
//!    mostly JSON lines (E08-S05), so JSON mode, its default, is on in every mode but `raw_`.
//!    Before each frame the log clock advances a frame (that frame's lines arrive; the service
//!    commits a batch on its tick), the view's pump applies the delta and its coalesced notify
//!    lands (drawing the window, as GPUI's next frame would); then the frame draws. Both draws go
//!    through the frame hook.
//!
//! 3. **Merged** (E08-S04): the same 5 000 lines a second as the log of a Deployment whose
//!    [`MERGED_PODS`] pods write one line in ten each, read through an aggregate session (ten
//!    streams, the pod watch, the merge by server timestamp, the pod gutters), following with wrap
//!    off (`merged_*`) and wrapped (`merged_wrap_*`). The merge holds a line for its reorder
//!    window (300 ms) before it commits it, so the lines in flight at any time are that many more.
//!
//! The sample fails (exit 1) unless every mode received lines at about the scripted rate and no
//! frame absorbed more than one coalesced notify.

use std::rc::Rc;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail, ensure};
use gpui::{
    AnyView, AnyWindowHandle, AppContext as _, Context, Entity, HeadlessAppContext, IntoElement,
    Render, Window,
};
use oxikube_app::logs::{LogConfig, LogService};
use oxikube_domain::log::LevelChip;
use oxikube_logs_ui::view::ViewOptions;
use oxikube_logs_ui::{LogView, LogViewDeps, SearchMode, log_runtime};
use oxikube_runtime::perf::harness::{self, metric};
use oxikube_runtime::perf::{PerfRoot, Recorder, ScenarioSample, Summary};
use oxikube_testkit::headless;

use super::{FRAMES, WINDOW_SIZE};

mod fixture;

use fixture::{FRAME, LINES_PER_S, MERGED_PODS, TAIL};

/// The scenario's name.
pub(super) const NAME: &str = "logs-stream";

/// Overrides the scripted frames per mode, [`FRAMES`] (at most that many): the smoke test runs a
/// debug build with a few frames.
const FRAMES_ENV: &str = "OXIKUBE_PERF_LOGS_FRAMES";

/// Upper bound on the frames the tail may take to show.
const MAX_TURNS: usize = 240;

/// The search of the `search_` and `filter_` modes: the fixture's warnings and errors (about one
/// line in six), a regex with an alternation.
const PATTERN: &str = "WARN|ERROR";

/// One measured mode: the prefix of its metrics, wrap, autoscroll, JSON mode, the level chips that
/// are off, and the search it runs with.
struct Mode {
    prefix: &'static str,
    wrap: bool,
    follow: bool,
    json: bool,
    hidden: &'static [LevelChip],
    search: Option<SearchMode>,
}

impl Mode {
    const fn new(prefix: &'static str, wrap: bool, follow: bool) -> Self {
        Self {
            prefix,
            wrap,
            follow,
            json: true,
            hidden: &[],
            search: None,
        }
    }
}

/// The modes, in run order; the first is the budget's (no prefix). JSON mode is on (its default)
/// in all but `raw_`, over a stream of mostly JSON lines, so the columns and the per-line level
/// are in every figure; `json_filtered_` also hides the debug and plain-text lines.
const MODES: [Mode; 8] = [
    Mode::new("", false, true),
    Mode::new("paused_", false, false),
    Mode::new("wrap_", true, true),
    Mode::new("wrap_paused_", true, false),
    Mode {
        json: false,
        ..Mode::new("raw_", false, true)
    },
    Mode {
        hidden: &[LevelChip::Debug, LevelChip::Text],
        ..Mode::new("json_filtered_", false, true)
    },
    Mode {
        search: Some(SearchMode::Highlight),
        ..Mode::new("search_", false, true)
    },
    Mode {
        search: Some(SearchMode::Filter),
        ..Mode::new("filter_", false, true)
    },
];

/// The merged variant's modes: following, wrap off and on.
const MERGED_MODES: [Mode; 2] = [
    Mode::new("merged_", false, true),
    Mode::new("merged_wrap_", true, true),
];

/// Lines in flight in the merged variant beyond the service's tick: the reorder window's worth.
const MERGE_WINDOW_MS: u64 = 300;

/// The frames to script per mode: [`FRAMES`], or fewer from [`FRAMES_ENV`].
fn frames() -> usize {
    std::env::var(FRAMES_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(FRAMES, |n: usize| n.clamp(1, FRAMES))
}

/// One sample. `probe` puts the `--perf` frame hook in the window.
pub(super) fn run(probe: bool) -> Result<ScenarioSample> {
    let recorder = Arc::new(Recorder::new());
    // The process-wide recorder: `notify_coalesced` reports to it.
    oxikube_runtime::perf::install(recorder.clone());
    let frames = frames();
    // Every mode's frames, the tail's turns and the settling frame between modes.
    let turns_budget = MAX_TURNS + (MODES.len() + MERGED_MODES.len()) * (frames + 1);
    let single = fixture::Fixture::connected(turns_budget)?;
    let merged = fixture::Fixture::merged(turns_budget)?;

    let mut cx = headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets));
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
    });

    // 1. and 2. One pod: the tail, then the modes.
    let one = Stage::open(&mut cx, &single, &recorder, probe, |target, deps, cx| {
        LogView::new(target, None, deps, cx)
    })?;
    let turns = one.wait_for_tail(&mut cx)?;
    let mut main = None;
    let mut extra = Vec::new();
    for mode in &MODES {
        let mut run = one.run_mode(&mut cx, &recorder, mode, frames, LINES_PER_S * 40 / 1_000)?;
        if mode.prefix.is_empty() {
            main = Some(run);
        } else {
            extra.extend(extra_metrics(mode, &mut run));
        }
    }

    // 3. A Deployment of ten pods, merged. The window of the merge is in flight on top of the
    // service's tick.
    let many = Stage::open(&mut cx, &merged, &recorder, probe, |target, deps, cx| {
        LogView::workload(target, ViewOptions::default(), deps, cx)
    })?;
    many.wait_for_tail(&mut cx)?;
    let in_flight = LINES_PER_S * (40 + MERGE_WINDOW_MS) / 1_000;
    for mode in &MERGED_MODES {
        let mut run = many.run_mode(&mut cx, &recorder, mode, frames, in_flight)?;
        extra.extend(extra_metrics(mode, &mut run));
    }

    let main = main.context("the budget's mode ran")?;
    eprintln!(
        "oxikube logs-stream: tail on screen after {turns} frames; {} frames, {} notifies (at \
         most {} per frame) in the budget's mode; merged over {MERGED_PODS} pods too",
        main.counters.frames, main.counters.notifies, main.counters.max_notifies_per_frame
    );
    Ok(main.into_sample(NAME, extra))
}

/// The frame and draw summaries of a mode that is not the budget's, named by its prefix.
fn extra_metrics(mode: &Mode, run: &mut harness::FrameRun) -> Vec<(String, Summary)> {
    [
        (metric::FRAME_MS, &mut run.frame_ns),
        (metric::DRAW_MS, &mut run.draw_ns),
    ]
    .into_iter()
    .filter_map(|(name, samples)| {
        Summary::from_nanos(samples).map(|summary| (format!("{}{name}", mode.prefix), summary))
    })
    .collect()
}

/// One headless window with a log view over one fixture: the log clock, the view and the window
/// the modes draw in.
struct Stage {
    view: Entity<LogView>,
    window: AnyWindowHandle,
    log_clock: Arc<oxikube_testkit::FakeClockPort>,
}

impl Stage {
    /// Opens the window with the view `make` builds over `fixture`; `probe` puts the frame hook
    /// in it.
    fn open(
        cx: &mut HeadlessAppContext,
        fixture: &fixture::Fixture,
        recorder: &Arc<Recorder>,
        probe: bool,
        make: impl FnOnce(
            oxikube_domain::ids::ResourceRef,
            LogViewDeps,
            &mut Context<LogView>,
        ) -> LogView,
    ) -> Result<Self> {
        let log_clock = fixture.log_clock();
        let service = cx.update(|cx| {
            Arc::new(LogService::new(
                log_runtime(log_clock.clone(), cx),
                LogConfig::default(),
            ))
        });
        let deps = LogViewDeps {
            service,
            sessions: fixture.sessions.clone(),
            dispatcher: Rc::new(fixture::Ignore),
            fs: Arc::new(oxikube_testkit::FakeFsPort::new()),
            agent: oxikube_app::context::PendingContext::new(),
        };
        let hook = probe.then(|| recorder.clone());
        let target = fixture.target.clone();
        let mut view: Option<Entity<LogView>> = None;
        let window: AnyWindowHandle = cx
            .open_window(WINDOW_SIZE, |_, cx| {
                let log = cx.new(|cx| make(target, deps, cx));
                view = Some(log.clone());
                let content: AnyView = match hook {
                    Some(recorder) => cx.new(|_| PerfRoot::new(log, recorder)).into(),
                    None => log.into(),
                };
                cx.new(|_| Root(content))
            })?
            .into();
        Ok(Self {
            view: view.context("the log view")?,
            window,
            log_clock,
        })
    }

    /// One frame of the scenario: the log clock moves a frame (that frame's lines arrive) and
    /// the app's executor settles, then GPUI's next frame.
    fn park(&self, cx: &HeadlessAppContext) {
        self.log_clock.advance(FRAME);
        cx.run_until_parked();
        cx.advance_clock(FRAME);
        cx.run_until_parked();
    }

    /// Runs frames until the tail is on screen; the number of frames it took.
    fn wait_for_tail(&self, cx: &mut HeadlessAppContext) -> Result<usize> {
        let mut turns = 0;
        while read(&self.view, cx, |v| v.line_window().line_count()) < TAIL {
            ensure!(turns < MAX_TURNS, "gave up waiting for the tail to show");
            self.park(cx);
            turns += 1;
        }
        Ok(turns)
    }

    /// Switches to `mode` and runs `frames` scripted frames of it.
    fn run_mode(
        &self,
        cx: &mut HeadlessAppContext,
        recorder: &Arc<Recorder>,
        mode: &Mode,
        frames: usize,
        in_flight: u64,
    ) -> Result<harness::FrameRun> {
        harness::draw_frame(cx, self.window, |window, cx| {
            self.view.update(cx, |v, cx| {
                if v.options().wrap != mode.wrap {
                    v.toggle_wrap(cx);
                }
                if v.autoscroll() != mode.follow {
                    v.toggle_autoscroll(cx);
                }
                if v.options().json != mode.json {
                    v.toggle_json_mode(cx);
                }
                for chip in LevelChip::ALL {
                    if v.levels().shows(chip) == mode.hidden.contains(&chip) {
                        v.toggle_level(chip, cx);
                    }
                }
                if let Some(search) = mode.search {
                    if !v.search_state().is_open() {
                        v.find(Some(PATTERN), window, cx);
                    }
                    if v.search_state().mode() != search {
                        v.toggle_filter_mode(cx);
                    }
                }
            });
        })?;
        self.park(cx);
        ensure!(
            read(&self.view, cx, |v| v.options().wrap == mode.wrap
                && v.autoscroll() == mode.follow
                && v.options().json == mode.json
                && mode
                    .search
                    .is_none_or(|search| v.search_state().mode() == search
                        && v.line_window().index().is_some())),
            "the view did not switch to mode `{}`",
            mode.prefix
        );
        let mut reader = recorder.reader();
        reader.drain(recorder);
        let before = read(&self.view, cx, |v| v.line_window().next_seq());
        let run = harness::run_frames(
            cx,
            self.window,
            frames,
            recorder,
            &mut reader,
            |cx| self.park(cx),
            |_, _, _| {},
        )?;
        let received = read(&self.view, cx, |v| v.line_window().next_seq()) - before;
        check(mode, &run, received, frames, in_flight)?;
        Ok(run)
    }
}

/// Fails the sample unless the mode received lines at about the scripted rate and its notifies
/// were coalesced to frame cadence. `in_flight` lines may still be in a batch (or, merged, in the
/// reorder window).
fn check(
    mode: &Mode,
    run: &harness::FrameRun,
    received: u64,
    frames: usize,
    in_flight: u64,
) -> Result<()> {
    let name = if mode.prefix.is_empty() {
        "wrap off, following"
    } else {
        mode.prefix.trim_end_matches('_')
    };
    let written = fixture::streamed_lines(frames) as u64;
    ensure!(
        received + in_flight >= written,
        "{name}: the view received {received} lines of the {written} written"
    );
    if run.counters.max_notifies_per_frame > 1 {
        bail!(
            "{name}: {} coalesced notifies landed between two frames: the log view's notify \
             path is not coalesced to frame cadence",
            run.counters.max_notifies_per_frame
        );
    }
    eprintln!(
        "oxikube logs-stream ({name}): {received} lines in {frames} frames (~{} lines/s)",
        received * 1_000_000 / (FRAME.as_micros() as u64 * frames as u64).max(1)
    );
    Ok(())
}

/// Reads the view.
fn read<R>(
    view: &Entity<LogView>,
    cx: &mut HeadlessAppContext,
    f: impl FnOnce(&LogView) -> R,
) -> R {
    cx.update(|cx| f(view.read(cx)))
}

/// The window's root: the view, behind the frame hook when probing.
struct Root(AnyView);

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0.clone()
    }
}
