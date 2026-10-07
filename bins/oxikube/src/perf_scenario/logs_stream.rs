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
use oxikube_logs_ui::{LogView, LogViewDeps, SearchMode, log_runtime};
use oxikube_runtime::perf::harness::{self, metric};
use oxikube_runtime::perf::{PerfRoot, Recorder, ScenarioSample, Summary};
use oxikube_testkit::headless;

use super::{FRAMES, WINDOW_SIZE};

mod fixture;

use fixture::{FRAME, LINES_PER_S, TAIL};

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
    let fixture = fixture::Fixture::connected(MAX_TURNS + MODES.len() * (frames + 1))?;
    let log_clock = fixture.log_clock();

    let mut cx = headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets));
    let service = cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
        Arc::new(LogService::new(
            log_runtime(log_clock.clone(), cx),
            LogConfig::default(),
        ))
    });
    let deps = LogViewDeps {
        service,
        sessions: fixture.sessions.clone(),
        dispatcher: Rc::new(fixture::Ignore),
    };

    // 1. The tail.
    let hook = probe.then(|| recorder.clone());
    let mut view: Option<Entity<LogView>> = None;
    let target = fixture.target;
    let window: AnyWindowHandle = cx
        .open_window(WINDOW_SIZE, |_, cx| {
            let log = cx.new(|cx| LogView::new(target, None, deps, cx));
            view = Some(log.clone());
            let content: AnyView = match hook {
                Some(recorder) => cx.new(|_| PerfRoot::new(log, recorder)).into(),
                None => log.into(),
            };
            cx.new(|_| Root(content))
        })?
        .into();
    let view = view.context("the log view")?;
    let park = |cx: &HeadlessAppContext| {
        log_clock.advance(FRAME);
        cx.run_until_parked();
        cx.advance_clock(FRAME);
        cx.run_until_parked();
    };
    let mut turns = 0;
    while read(&view, &mut cx, |v| v.line_window().line_count()) < TAIL {
        ensure!(turns < MAX_TURNS, "gave up waiting for the tail to show");
        park(&cx);
        turns += 1;
    }

    // 2. The modes.
    let mut main = None;
    let mut extra = Vec::new();
    for mode in &MODES {
        harness::draw_frame(&mut cx, window, |window, cx| {
            view.update(cx, |v, cx| {
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
        park(&cx);
        ensure!(
            read(&view, &mut cx, |v| v.options().wrap == mode.wrap
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
        reader.drain(&recorder);
        let before = read(&view, &mut cx, |v| v.line_window().next_seq());
        let mut run = harness::run_frames(
            &mut cx,
            window,
            frames,
            &recorder,
            &mut reader,
            park,
            |_, _, _| {},
        )?;
        let received = read(&view, &mut cx, |v| v.line_window().next_seq()) - before;
        check(mode, &run, received, frames)?;
        if mode.prefix.is_empty() {
            main = Some(run);
        } else {
            for (name, samples) in [
                (metric::FRAME_MS, &mut run.frame_ns),
                (metric::DRAW_MS, &mut run.draw_ns),
            ] {
                if let Some(summary) = Summary::from_nanos(samples) {
                    extra.push((format!("{}{name}", mode.prefix), summary));
                }
            }
        }
    }
    let main = main.context("the budget's mode ran")?;
    eprintln!(
        "oxikube logs-stream: tail on screen after {turns} frames; {} frames, {} notifies (at \
         most {} per frame) in the budget's mode",
        main.counters.frames, main.counters.notifies, main.counters.max_notifies_per_frame
    );
    Ok(main.into_sample(NAME, extra))
}

/// Fails the sample unless the mode received lines at about the scripted rate and its notifies
/// were coalesced to frame cadence.
fn check(mode: &Mode, run: &harness::FrameRun, received: u64, frames: usize) -> Result<()> {
    let name = if mode.prefix.is_empty() {
        "wrap off, following"
    } else {
        mode.prefix.trim_end_matches('_')
    };
    let written = fixture::streamed_lines(frames) as u64;
    // The service commits on its 32 ms tick, so up to a tick of lines is still in a batch.
    let in_flight = LINES_PER_S * 40 / 1_000;
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
