//! The `notifications-burst` scenario (E14-S05): a burst of 1 000 Warning events in one second
//! reaching the notification centre of a window built as the app builds its own ([`window_root`]),
//! with the centre's panel open, its status-bar bell and its toasts.
//!
//! The burst takes the shipped path end to end on testkit fakes: `FakeEventsPort` feeds a
//! connected cluster's `EventService` (its one feed), whose warnings subscription the
//! `NotificationService` pumps through its tracker (what is news), the rules (watched kinds,
//! ignored reasons), grouping, dedupe and the rate limit into the centre; the window's
//! `NotificationCentre` derives its rows and redraws through `notify_coalesced`, delivered at
//! frame cadence (each frame advances the clock by one frame, so the pacer runs as in the app).
//!
//! 1. **Burst** (`frame_ms` / `draw_ms`, the budget's metrics): [`FRAMES`] frames (one second at
//!    120 Hz), one feed tick each, over which 1 000 warnings about distinct pods arrive (50
//!    reasons), about nine per frame. The first list held [`BACKLOG`] older warnings, which must
//!    not notify.
//! 2. **Idle** (`idle_frame_ms` / `idle_draw_ms`): the same number of frames with nothing new, the
//!    panel showing the burst's 200 rows (50 reasons: three rows of their own and a group each).
//!
//! The sample fails when more than `MAX_TOASTS` toasts plus the summary are ever on screen or
//! waiting (a burst must never stack toasts), when a frame absorbed more than one coalesced
//! notify, when no notify reached the window, when the status-bar count is not the centre's, or
//! when the centre does not hold every warning of the burst.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use gpui::{AnyWindowHandle, AppContext as _, Entity, HeadlessAppContext};
use jiff::{SignedDuration, Timestamp};
use oxikube_app::ClusterSessionManager;
use oxikube_app::events::{EventFilter, EventService, EventsConfig};
use oxikube_app::notifications::NotificationService;
use oxikube_app::store::Spawner;
use oxikube_domain::command::Command;
use oxikube_domain::event::Event;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, Delta, SourceId};
use oxikube_runtime::perf::harness::{self, FrameRun};
use oxikube_runtime::perf::{Recorder, ScenarioSample, Summary};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeEventFeed, FakeEventsPort,
    FakeStatePort, TICK, event, headless,
};
use oxikube_workspace::notifications::{MAX_TOASTS, NotificationDeps, NotificationPanel};
use oxikube_workspace::{CommandDispatcher, Workspace};

use super::{FRAMES, WINDOW_SIZE, window_root};

/// The scenario's name.
pub(super) const NAME: &str = "notifications-burst";

/// The burst: this many warnings over [`FRAMES`] frames.
const BURST: usize = 1_000;
/// Older warnings the cluster's first list holds: the backlog, never news.
const BACKLOG: usize = 200;
/// One frame at 120 Hz.
const FRAME: Duration = Duration::from_micros(8_333);
/// Upper bound on the frames the warm-up may take.
const MAX_TURNS: usize = 240;

/// Overrides the frames per phase (a debug build in the smoke test uses a few).
const FRAMES_ENV: &str = "OXIKUBE_PERF_NOTIFICATIONS_FRAMES";

fn frames() -> usize {
    std::env::var(FRAMES_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(FRAMES, |n: usize| n.clamp(1, FRAMES))
}

/// The centre's clock: the burst happens now.
fn now() -> Timestamp {
    Timestamp::UNIX_EPOCH + SignedDuration::from_secs(1_790_000_000)
}

/// The bus stand-in: the scenario measures the centre, not the commands.
struct NoBus;

impl CommandDispatcher for NoBus {
    fn dispatch(&self, _: Command, _: &mut gpui::App) {}
}

/// A Warning `uid` about pod `name`, one of 50 reasons by `i`, last seen `at`.
fn warning(cluster: &ClusterId, uid: &str, name: &str, i: usize, at: Timestamp) -> Event {
    let mut e = event()
        .uid(uid)
        .warning()
        .reason(&format!("Reason{}", i % 50))
        .message("Back-off restarting failed container web in pod")
        .regarding("Pod", Some("default"), name, &format!("uid-{name}"))
        .build();
    e.regarding.cluster = cluster.clone();
    e.last_seen = Some(at);
    e
}

/// The feed: [`BACKLOG`] warnings an hour old listed first, then the burst over `ticks` ticks,
/// each warning about a distinct pod and last seen a millisecond after the one before.
fn feed(cluster: &ClusterId, ticks: usize) -> FakeEventFeed {
    let hour_ago = now() - SignedDuration::from_hours(1);
    let backlog = (0..BACKLOG).map(|i| {
        let name = format!("old-{i}");
        warning(cluster, &format!("backlog-{i}"), &name, i, hour_ago)
    });
    let mut feed = FakeEventFeed::new().initial(backlog);
    let per_tick = BURST.div_ceil(ticks);
    for tick in 0..ticks {
        let burst = (tick * per_tick..((tick + 1) * per_tick).min(BURST)).map(|i| {
            let at = now() + SignedDuration::from_millis(i as i64);
            Delta::Applied(warning(
                cluster,
                &format!("burst-{i}"),
                &format!("web-{i}"),
                i,
                at,
            ))
        });
        feed = feed.at(tick as u32 + 1, burst);
    }
    feed
}

/// Runs the executor until idle, a frame's worth of clock passing in between so the
/// coalesced notifies are delivered as in the app.
fn settle(cx: &HeadlessAppContext) {
    cx.run_until_parked();
    cx.advance_clock(FRAME);
    cx.run_until_parked();
}

/// A burst frame: the feed's next tick, then [`settle`].
fn tick(port: &FakeEventsPort, cx: &HeadlessAppContext) {
    port.clock().advance(TICK);
    settle(cx);
}

/// One sample. `probe` puts the `--perf` frame hook in the window.
pub(super) fn run(probe: bool) -> Result<ScenarioSample> {
    let recorder = Arc::new(Recorder::new());
    oxikube_runtime::perf::install(recorder.clone());
    let frames = frames();

    let context = ContextName::new("burst");
    let cluster = ClusterId::new("/perf/kubeconfig", &context);
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let port = connector.ports_for(&cluster).events;
    port.install(feed(&cluster, frames));
    let source = Arc::new(
        FakeClusterSourcePort::new().with_contexts([ClusterContext::new(
            cluster.clone(),
            context,
            SourceId("kubeconfig".into()),
        )]),
    );
    let clock = Arc::new(FakeClockPort::new(now()));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster)).context("connecting")?;

    let mut cx = headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets));
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
    });
    let spawner: Arc<dyn Spawner> = cx.update(|cx| {
        let executor = cx.background_executor().clone();
        Arc::new(move |task| executor.spawn(task).detach()) as Arc<dyn Spawner>
    });
    let events = EventService::new(sessions.clone(), clock.clone(), EventsConfig::default());
    events.start(spawner.clone());
    let service = NotificationService::new(
        events.clone(),
        sessions.clone(),
        Arc::new(FakeStatePort::new()),
        clock,
    );
    service.start(spawner);

    let hook = probe.then(|| recorder.clone());
    let mut workspace: Option<Entity<Workspace>> = None;
    let mut text_font = None;
    let window_handle: AnyWindowHandle = cx
        .open_window(WINDOW_SIZE, |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            let (root, font) = window_root::mount(view.into(), hook, window, cx);
            text_font = Some(font);
            root
        })?
        .into();
    let workspace = workspace.context("the workspace")?;
    let deps = NotificationDeps {
        service: service.clone(),
        sessions,
        dispatcher: Rc::new(NoBus),
    };
    let wiring = cx.update_window(window_handle, |_, window, cx| {
        let wiring = oxikube_workspace::notifications::install(&workspace, deps, window, cx);
        workspace.update(cx, |ws, cx| {
            ws.toggle_panel::<NotificationPanel>(window, cx)
        });
        wiring
    })?;

    // Warm: the centre pumps the cluster's one feed, whose first list (the backlog) is in.
    let mut turns = 0;
    while !(service.is_watching(&cluster)
        && port.live_feeds() == 1
        && events.query(&cluster, &EventFilter::warnings()).len() == BACKLOG)
    {
        ensure!(turns < MAX_TURNS, "gave up waiting for the cluster's feed");
        harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
        settle(&cx);
        turns += 1;
    }
    harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
    settle(&cx);
    cx.update(|cx| text_font.context("the window root")?.check(cx))?;
    ensure!(
        service.list().is_empty(),
        "the backlog of {BACKLOG} warnings notified"
    );

    let mut most_toasts = 0;
    let phase = |cx: &mut HeadlessAppContext,
                 park: &dyn Fn(&HeadlessAppContext),
                 step: &mut dyn FnMut(usize, &mut gpui::Window, &mut gpui::App)|
     -> Result<FrameRun> {
        let mut reader = recorder.reader();
        reader.drain(&recorder);
        let run = harness::run_frames(
            cx,
            window_handle,
            frames,
            &recorder,
            &mut reader,
            park,
            step,
        )?;
        if run.counters.max_notifies_per_frame > 1 {
            bail!(
                "{} coalesced notifies landed between two frames: the centre's notify path is \
                 not coalesced to frame cadence",
                run.counters.max_notifies_per_frame
            );
        }
        Ok(run)
    };

    // 1. The burst: one feed tick (about nine warnings) per frame.
    let main = phase(&mut cx, &|cx| tick(&port, cx), &mut |_, _, cx| {
        let layer = workspace.read(cx).toast_layer().read(cx);
        most_toasts = most_toasts.max(layer.visible().len() + layer.pending_len());
    })?;

    // 2. Idle, the panel full.
    let mut idle = phase(&mut cx, &settle, &mut |_, _, _| {})?;

    let (rows, shown_unread) = cx.update(|cx| {
        (
            wiring.centre.read(cx).rows().len(),
            wiring.status.read(cx).unread(),
        )
    });
    let listed = service.list();
    let told: u64 = listed.iter().map(|n| u64::from(n.count)).sum();
    ensure!(
        most_toasts <= MAX_TOASTS + 1,
        "{most_toasts} toasts for a burst: the cap is {MAX_TOASTS} plus one summary"
    );
    ensure!(
        main.counters.notifies > 0,
        "no coalesced notify reached the window during the burst"
    );
    ensure!(
        rows == listed.len() && shown_unread == service.unread(),
        "the window is behind the centre: {rows} rows and {shown_unread} unread shown, {} and {} \
         held",
        listed.len(),
        service.unread()
    );
    ensure!(
        told == BURST as u64,
        "{told} of the {BURST} warnings reached the centre"
    );
    let extra: Vec<(String, Summary)> = [
        (harness::metric::FRAME_MS, &mut idle.frame_ns),
        (harness::metric::DRAW_MS, &mut idle.draw_ns),
    ]
    .into_iter()
    .filter_map(|(name, samples)| Summary::from_nanos(samples).map(|s| (format!("idle_{name}"), s)))
    .collect();
    eprintln!(
        "oxikube notifications-burst: {BURST} warnings over {} frames, {rows} rows listed, \
         {shown_unread} unread, at most {most_toasts} toasts; {} notifies (at most {} per frame)",
        main.counters.frames, main.counters.notifies, main.counters.max_notifies_per_frame
    );
    Ok(main.into_sample(NAME, extra))
}
