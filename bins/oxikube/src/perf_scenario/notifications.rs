//! The `notifications-burst` scenario (E14-S05): a burst of 1 000 Warning events in one second
//! reaching the notification centre of a window built as the app builds its own ([`window_root`]),
//! with the centre's panel open, its status-bar bell and its toasts.
//!
//! 1. **Burst** (`frame_ms` / `draw_ms`, the budget's metrics): [`FRAMES`] frames (one second at
//!    120 Hz) over which 1 000 warnings about distinct pods arrive through the service's rules
//!    (50 reasons), about nine per frame: grouping, dedupe, the rate limit, the coalesced redraw of
//!    the panel and the badge, the toasts.
//! 2. **Idle** (`idle_frame_ms` / `idle_draw_ms`): the same number of frames with nothing new, the
//!    panel showing the burst's 200 rows (50 reasons: three rows of their own and a group each).
//!
//! The sample fails when more than `MAX_TOASTS` toasts plus the summary are ever on screen or
//! waiting: a burst must never stack toasts.

use std::rc::Rc;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use gpui::{AnyWindowHandle, AppContext as _, Entity};
use oxikube_app::events::{EventService, EventsConfig};
use oxikube_app::notifications::{NotificationRules, NotificationService};
use oxikube_app::{ClusterSessionManager, session::SessionOptions};
use oxikube_domain::command::Command;
use oxikube_domain::event::Event;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_runtime::perf::harness::{self, FrameRun};
use oxikube_runtime::perf::{Recorder, ScenarioSample, Summary};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort, event, headless,
};
use oxikube_workspace::notifications::{MAX_TOASTS, NotificationDeps, NotificationPanel};
use oxikube_workspace::{CommandDispatcher, Workspace};

use super::{FRAMES, WINDOW_SIZE, window_root};

/// The scenario's name.
pub(super) const NAME: &str = "notifications-burst";

/// The burst: this many warnings over [`FRAMES`] frames.
const BURST: usize = 1_000;

/// Overrides the frames per phase (a debug build in the smoke test uses a few).
const FRAMES_ENV: &str = "OXIKUBE_PERF_NOTIFICATIONS_FRAMES";

fn frames() -> usize {
    std::env::var(FRAMES_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(FRAMES, |n: usize| n.clamp(1, FRAMES))
}

/// The bus stand-in: the scenario measures the centre, not the commands.
struct NoBus;

impl CommandDispatcher for NoBus {
    fn dispatch(&self, _: Command, _: &mut gpui::App) {}
}

/// Warning `i` of the burst: a distinct pod, one of 50 reasons.
fn warning(cluster: &ClusterId, i: usize) -> Event {
    let mut e = event()
        .uid(&format!("burst-{i}"))
        .warning()
        .reason(&format!("Reason{}", i % 50))
        .message("Back-off restarting failed container web in pod")
        .regarding(
            "Pod",
            Some("default"),
            &format!("web-{i}"),
            &format!("uid-{i}"),
        )
        .build();
    e.regarding.cluster = cluster.clone();
    e
}

/// One sample. `probe` puts the `--perf` frame hook in the window.
pub(super) fn run(probe: bool) -> Result<ScenarioSample> {
    let recorder = Arc::new(Recorder::new());
    oxikube_runtime::perf::install(recorder.clone());
    let frames = frames();

    let cluster = ClusterId::new("perf", &ContextName::new("burst"));
    let context = ClusterContext::new(
        cluster.clone(),
        ContextName::new("burst"),
        SourceId("kubeconfig".into()),
    );
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new().with_contexts([context.clone()])),
        clock.clone(),
    );
    sessions.open(&context, SessionOptions::default());
    let events = EventService::new(sessions.clone(), clock.clone(), EventsConfig::default());
    let service = NotificationService::new(
        events,
        sessions.clone(),
        Arc::new(FakeStatePort::new()),
        clock,
    );
    let rules = NotificationRules::default();

    let mut cx = headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets));
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
    });
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
    harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
    harness::draw_frame(&mut cx, window_handle, |_, _| {})?;
    cx.update(|cx| text_font.context("the window root")?.check(cx))?;

    let mut most_toasts = 0;
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

    // 1. The burst: about nine warnings per frame.
    let per_frame = BURST.div_ceil(frames);
    let main = phase(&mut cx, &mut |frame, _, cx| {
        for i in frame * per_frame..((frame + 1) * per_frame).min(BURST) {
            let e = warning(&cluster, i);
            service.submit(rules.input_for(&cluster, &e, 1));
        }
        let layer = workspace.read(cx).toast_layer().read(cx);
        most_toasts = most_toasts.max(layer.visible().len() + layer.pending_len());
    })?;

    // 2. Idle, the panel full.
    let mut idle = phase(&mut cx, &mut |_, _, _| {})?;

    let rows = cx.update(|cx| wiring.centre.read(cx).rows().len());
    anyhow::ensure!(
        most_toasts <= MAX_TOASTS + 1,
        "{most_toasts} toasts for a burst: the cap is {MAX_TOASTS} plus one summary"
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
         {} unread, at most {most_toasts} toasts",
        main.counters.frames,
        service.unread()
    );
    Ok(main.into_sample(NAME, extra))
}
