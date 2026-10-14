//! The `events-stream` scenario (E14-S03): the events view over a full 10 000-event buffer while
//! [`PER_FRAME`] events arrive every frame (240 a second at 120 Hz, above the story's 200/s).
//!
//! It runs the real pieces behind the view on testkit fakes: a connected `ClusterSessionManager`,
//! the app's `EventService` (its buffer at the default 10 000) fed by `FakeEventsPort`, whose
//! timeline is the initial 10 000 events and then the stream, one tick per scripted frame, and
//! the [`EventsView`] in a headless window built as the app builds its own ([`window_root`]).
//!
//! 1. **Warm**: frames run until the 10 000 rows are on screen (read off the UI thread).
//! 2. **Live** (`frame_ms` / `draw_ms`, the budget's metrics): [`FRAMES`] frames, the live tail
//!    putting each frame's events on top.
//! 3. **Scroll** (`scroll_*`): the same stream while the rows scroll a page a frame (the tail
//!    pauses itself and counts what arrives).
//! 4. **Filter** (`filter_*`): a key typed into the text filter every frame (a regex on reason,
//!    message and object), each one a new subscription filter whose rows are read off the UI
//!    thread, the stream going on.
//! 5. **Faults** (`faults_*`): `ctrl-z`'s toggle every frame.
//! 6. **Burst** (`burst_*`): [`BURST`] events in one frame, [`BURST_FRAMES`] frames in a row.
//!    The feed's quiet ticks left before the bursts are delivered first, outside the measurement,
//!    so the phase's frames are exactly the burst ticks.
//!
//! The sample fails unless every live frame's events reached the rows, the last burst's newest
//! event is the top row, and no frame absorbed more than one coalesced notify.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use gpui::{AnyWindowHandle, AppContext as _, Entity, HeadlessAppContext};
use jiff::{SignedDuration, Timestamp};
use oxikube_app::ClusterSessionManager;
use oxikube_app::events::{EventService, EventsConfig};
use oxikube_app::store::Spawner;
use oxikube_domain::command::Command;
use oxikube_domain::event::{Event, EventType};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_events_ui::{EventsView, EventsViewDeps, Field};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_runtime::perf::harness::{self, FrameRun, metric};
use oxikube_runtime::perf::{Recorder, ScenarioSample, Summary};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeEventFeed, FakeEventsPort,
    FakeFsPort, FakeStatePort, TICK, event, headless,
};
use oxikube_workspace::CommandDispatcher;

use super::{FRAMES, WINDOW_SIZE, window_root};

/// The scenario's name.
pub(super) const NAME: &str = "events-stream";
/// Overrides the frames of each phase (clamped to `FRAMES`).
const FRAMES_ENV: &str = "OXIKUBE_PERF_EVENTS_FRAMES";

/// Events held: a full buffer at the default `events.buffer`.
const HELD: usize = 10_000;
/// Events arriving each frame of the streaming phases.
const PER_FRAME: usize = 2;
/// Events arriving in one frame of the burst phase.
const BURST: usize = 500;
/// Frames of the burst phase.
const BURST_FRAMES: usize = 20;
/// One frame at 120 Hz.
const FRAME: Duration = Duration::from_micros(8_333);
/// Upper bound on the frames the warm-up may take.
const MAX_TURNS: usize = 240;
/// What the filter phase types, a key a frame (cycled).
const TYPED: &str = "back(off|oom)";

fn at(seconds: usize) -> Timestamp {
    Timestamp::UNIX_EPOCH + SignedDuration::from_secs(1_790_000_000 + seconds as i64)
}

/// Event `n`: one in five a warning, ten namespaces, a few reasons, seen at second `n`.
fn nth(n: usize) -> Event {
    const REASONS: [&str; 5] = ["Pulled", "Created", "Started", "BackOff", "OOMKilled"];
    let ns = format!("team-{}", n % 10);
    let mut e = event()
        .uid(&format!("event-{n}"))
        .reason(REASONS[n % REASONS.len()])
        .message(&format!(
            "container app of pod web-{n} did something, attempt {n}"
        ))
        .regarding("Pod", Some(&ns), &format!("web-{}", n % 700), "uid")
        .build();
    if n.is_multiple_of(5) {
        e.event_type = EventType::Warning;
    }
    e.namespace = Some(ns.into());
    e.last_seen = Some(at(n));
    e
}

/// The uid of the newest event of [`feed`]`(ticks)` by `tick`.
fn newest(ticks: usize, tick: usize) -> String {
    let quiet = tick.min(ticks - BURST_FRAMES);
    let bursts = tick - quiet;
    format!("event-{}", HELD + quiet * PER_FRAME + bursts * BURST - 1)
}

/// The feed: [`HELD`] events, then `ticks` ticks of [`PER_FRAME`] and a burst at the end.
fn feed(ticks: usize) -> FakeEventFeed {
    let mut feed = FakeEventFeed::new().initial((0..HELD).map(nth));
    let mut n = HELD;
    for tick in 1..=ticks {
        let count = if tick > ticks - BURST_FRAMES {
            BURST
        } else {
            PER_FRAME
        };
        feed = feed.at(
            tick as u32,
            (0..count).map(|i| oxikube_ports::Delta::Applied(nth(n + i))),
        );
        n += count;
    }
    feed
}

/// The view's commands go nowhere in the scenario.
struct Ignore;

impl CommandDispatcher for Ignore {
    fn dispatch(&self, _: Command, _: &mut gpui::App) {}
}

struct Stage {
    view: Entity<EventsView>,
    window: AnyWindowHandle,
    port: Arc<FakeEventsPort>,
}

impl Stage {
    fn park(&self, cx: &HeadlessAppContext) {
        self.port.clock().advance(TICK);
        cx.run_until_parked();
        cx.advance_clock(FRAME);
        cx.run_until_parked();
    }

    /// Delivers the feed's quiet ticks until `newest` (the last event before the bursts) is the
    /// top row, then draws them, outside any measurement: the next tick is the first burst. The
    /// feed's ticks count from when the service opened it, so they are found by their events.
    fn drain_until(&self, newest: &str, cx: &mut HeadlessAppContext) -> Result<()> {
        let mut parks = 0;
        while self.rows(cx).1.as_deref() != Some(newest) {
            ensure!(
                parks < MAX_TURNS * 4,
                "gave up waiting for {newest}, the last event before the bursts"
            );
            self.park(cx);
            parks += 1;
        }
        harness::draw_frame(cx, self.window, |_, _| {})?;
        Ok(())
    }

    fn rows(&self, cx: &mut HeadlessAppContext) -> (usize, Option<String>) {
        cx.update(|cx| {
            let model = self.view.read(cx).model(cx);
            let top = model
                .events()
                .next()
                .and_then(|e| e.uid.as_deref().map(str::to_owned));
            (model.len(), top)
        })
    }

    fn phase(
        &self,
        cx: &mut HeadlessAppContext,
        recorder: &Arc<Recorder>,
        frames: usize,
        step: impl FnMut(usize, &mut gpui::Window, &mut gpui::App),
    ) -> Result<FrameRun> {
        let mut reader = recorder.reader();
        reader.drain(recorder);
        let run = harness::run_frames(
            cx,
            self.window,
            frames,
            recorder,
            &mut reader,
            |cx| self.park(cx),
            step,
        )?;
        if run.counters.max_notifies_per_frame > 1 {
            bail!(
                "{} coalesced notifies landed between two frames: the events view's notify path \
                 is not coalesced to frame cadence",
                run.counters.max_notifies_per_frame
            );
        }
        Ok(run)
    }
}

fn frames() -> usize {
    std::env::var(FRAMES_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(FRAMES, |n: usize| n.clamp(1, FRAMES))
}

fn extra(prefix: &str, run: &mut FrameRun) -> Vec<(String, Summary)> {
    [
        (metric::FRAME_MS, &mut run.frame_ns),
        (metric::DRAW_MS, &mut run.draw_ns),
    ]
    .into_iter()
    .filter_map(|(name, samples)| {
        Summary::from_nanos(samples).map(|s| (format!("{prefix}{name}"), s))
    })
    .collect()
}

/// One sample. `probe` puts the `--perf` frame hook in the window.
pub(super) fn run(probe: bool) -> Result<ScenarioSample> {
    let recorder = Arc::new(Recorder::new());
    oxikube_runtime::perf::install(recorder.clone());
    let frames = frames();
    let ticks = MAX_TURNS + 4 * (frames + 1) + BURST_FRAMES;

    let context = ContextName::new("perf");
    let cluster = ClusterId::new("/perf/kubeconfig", &context);
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let port = connector.ports_for(&cluster).events;
    port.install(feed(ticks));
    let source = Arc::new(
        FakeClusterSourcePort::new().with_contexts([ClusterContext::new(
            cluster.clone(),
            context,
            SourceId("kubeconfig".into()),
        )]),
    );
    let clock = Arc::new(FakeClockPort::new(at(HELD + ticks * BURST)));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster)).context("connecting")?;

    let mut cx = headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets));
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
    });
    let service = EventService::new(sessions.clone(), clock.clone(), EventsConfig::default());
    let spawner: Arc<dyn Spawner> = cx.update(|cx| {
        let executor = cx.background_executor().clone();
        Arc::new(move |task| executor.spawn(task).detach()) as Arc<dyn Spawner>
    });
    service.start(spawner);
    let deps = EventsViewDeps {
        service,
        sessions,
        dispatcher: Rc::new(Ignore),
        state: Arc::new(FakeStatePort::new()),
        fs: Arc::new(FakeFsPort::new()),
        clock,
    };
    let hook = probe.then(|| recorder.clone());
    let mut view = None;
    let mut text_font = None;
    let window: AnyWindowHandle = cx
        .open_window(WINDOW_SIZE, |window, cx| {
            let events = cx.new(|cx| EventsView::new(cluster, deps, None, window, cx));
            view = Some(events.clone());
            let (root, font) = window_root::mount(events.into(), hook, window, cx);
            text_font = Some(font);
            root
        })?
        .into();
    let stage = Stage {
        view: view.context("the events view")?,
        window,
        port,
    };

    // 1. Warm: the full buffer on screen.
    let mut turns = 0;
    while stage.rows(&mut cx).0 < HELD {
        ensure!(turns < MAX_TURNS, "gave up waiting for the {HELD} rows");
        harness::draw_frame(&mut cx, window, |_, _| {})?;
        stage.park(&cx);
        turns += 1;
    }
    cx.update(|cx| text_font.context("the window root")?.check(cx))?;

    // 2. Live.
    let before = stage.rows(&mut cx).1;
    let main = stage.phase(&mut cx, &recorder, frames, |_, _, _| {})?;
    let after = stage.rows(&mut cx).1;
    ensure!(before != after, "the live tail put no new event on top");
    let view = stage.view.clone();

    // 3. Scroll a page a frame.
    let mut scroll = stage.phase(&mut cx, &recorder, frames, |frame, _, cx| {
        view.update(cx, |v, cx| v.scroll_to((frame * 40) % HELD, cx));
    })?;
    view.update(&mut cx, |v, cx| v.toggle_pause(cx));

    // 4. Typing into the text filter.
    let typed: Vec<char> = TYPED.chars().collect();
    let mut filter = stage.phase(&mut cx, &recorder, frames, |frame, _, cx| {
        let text: String = typed[..=frame % typed.len()].iter().collect();
        view.update(cx, |v, cx| {
            v.set_field(Field::Text, text, cx);
        });
    })?;
    view.update(&mut cx, |v, cx| {
        v.set_field(Field::Text, String::new(), cx);
    });

    // 5. Faults only, on and off.
    let mut faults = stage.phase(&mut cx, &recorder, frames, |_, _, cx| {
        view.update(cx, |v, cx| v.toggle_faults(cx));
    })?;

    // 6. Bursts: the feed's last ticks, every event shown (faults off) and the tail live.
    if cx.update(|cx| view.read(cx).filters().faults_only) {
        view.update(&mut cx, |v, cx| v.toggle_faults(cx));
    }
    stage.drain_until(&newest(ticks, ticks - BURST_FRAMES), &mut cx)?;
    let mut burst = stage.phase(&mut cx, &recorder, BURST_FRAMES, |_, _, _| {})?;
    let (rows, top) = stage.rows(&mut cx);
    ensure!(
        rows == HELD && top.as_deref() == Some(newest(ticks, ticks).as_str()),
        "the bursts did not reach the rows: {rows} rows, {top:?} on top"
    );

    let mut metrics = extra("scroll_", &mut scroll);
    metrics.extend(extra("filter_", &mut filter));
    metrics.extend(extra("faults_", &mut faults));
    metrics.extend(extra("burst_", &mut burst));
    eprintln!(
        "oxikube events-stream: {HELD} events on screen after {turns} frames; {} frames, {} \
         notifies (at most {} per frame) streaming {PER_FRAME} events a frame",
        main.counters.frames, main.counters.notifies, main.counters.max_notifies_per_frame
    );
    Ok(main.into_sample(NAME, metrics))
}
