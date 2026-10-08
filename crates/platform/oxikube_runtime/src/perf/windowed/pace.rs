//! [`drive`]: one scripted phase, a step on every display refresh of the real window.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use futures::FutureExt as _;
use futures::channel::oneshot;
use gpui::{AnyWindowHandle, App, AsyncApp, Window};

use super::meter::Meter;
use super::summary::PhaseKind;

/// What a step tells the pacer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Call me on the next refresh (until the phase's time is up).
    Continue,
    /// The phase is done early (the script ran out).
    Stop,
}

/// What a step knows about where it is.
pub struct Step<'a> {
    /// Refreshes of this phase before this one.
    pub index: u64,
    /// Time since the phase began.
    pub elapsed: Duration,
    meter: &'a Meter,
}

impl Step<'_> {
    /// Marks that the step dispatches an input now (a keystroke, a scroll event, an action or a
    /// command): its latency runs to the end of the next frame. Call it right before the dispatch.
    pub fn input(&self) {
        self.meter.input();
    }
}

type StepFn = Box<dyn FnMut(&Step<'_>, &mut Window, &mut App) -> Result<Flow>>;

struct Pacer {
    step: StepFn,
    meter: Meter,
    started: Instant,
    /// Refreshes the window was called on (the stall check watches it move).
    refreshes: u64,
    duration: Duration,
    index: u64,
    done: Option<oneshot::Sender<Result<()>>>,
}

/// Runs phase `name` of the scenario: on every display refresh of `window` (GPUI's
/// `on_next_frame`, which the platform calls from its display link before drawing), `step` runs
/// and may change the UI through its real paths; GPUI then draws and presents the frame. Until
/// `duration` has passed or `step` returns [`Flow::Stop`].
///
/// Each refresh is reported to `meter` (with whether the window was the key window), so a refresh
/// the window missed shows as a gap: that is how dropped frames are counted. Because the pacer
/// keeps a next-frame callback registered, GPUI keeps asking the platform for frames for as long
/// as the phase runs, as it does for an animation.
///
/// # Errors
///
/// The step's error, or the window closing before the phase ended.
pub async fn drive(
    cx: &mut AsyncApp,
    window: AnyWindowHandle,
    meter: &Meter,
    name: &str,
    duration: Duration,
    step: impl FnMut(&Step<'_>, &mut Window, &mut App) -> Result<Flow> + 'static,
) -> Result<()> {
    let (done, finished) = oneshot::channel();
    meter.begin(name, PhaseKind::Driven);
    let pacer = Rc::new(RefCell::new(Pacer {
        step: Box::new(step),
        meter: meter.clone(),
        started: Instant::now(),
        refreshes: 0,
        duration,
        index: 0,
        done: Some(done),
    }));
    let scheduled = pacer.clone();
    window
        .update(cx, |_, window, _| schedule(scheduled, window))
        .context("the scenario's window is gone")?;
    let mut finished = finished.fuse();
    let (mut seen, mut quiet) = (0, Duration::ZERO);
    loop {
        let mut tick = cx.background_executor().timer(STALL_CHECK).fuse();
        futures::select_biased! {
            result = finished => {
                return result.map_err(|_| anyhow!("the window closed during phase `{name}`"))?;
            }
            _ = tick => {
                let refreshes = pacer.borrow().refreshes;
                quiet = if refreshes == seen { quiet + STALL_CHECK } else { Duration::ZERO };
                seen = refreshes;
                if quiet >= STALL {
                    pacer.borrow_mut().done.take();
                    bail!(
                        "the window got no display refresh for {:.1} s in phase `{name}`: it is \
                         hidden, minimised or on another Space (macOS stops refreshing a window \
                         nobody can see); keep it in front for the whole run",
                        quiet.as_secs_f64()
                    );
                }
            }
        }
    }
}

/// How long a phase may go without a display refresh before the run gives up.
const STALL: Duration = Duration::from_secs(5);
/// How often the stall is checked.
const STALL_CHECK: Duration = Duration::from_millis(500);

/// Runs idle phase `name`: nothing is driven for `duration`; the frames the app draws on its own
/// and the CPU it spends are what is measured.
pub async fn idle(cx: &mut AsyncApp, meter: &Meter, name: &str, duration: Duration) {
    meter.begin(name, PhaseKind::Idle);
    cx.background_executor().timer(duration).await;
}

fn schedule(pacer: Rc<RefCell<Pacer>>, window: &Window) {
    window.on_next_frame(move |window, cx| refresh(pacer, window, cx));
}

fn refresh(pacer: Rc<RefCell<Pacer>>, window: &mut Window, cx: &mut App) {
    let now = Instant::now();
    let flow = {
        let mut p = pacer.borrow_mut();
        if p.done.is_none() {
            // The phase was abandoned (a stall): stop calling the step.
            return;
        }
        p.refreshes += 1;
        p.meter.refresh(now, window.is_window_active());
        let elapsed = now.saturating_duration_since(p.started);
        if elapsed >= p.duration {
            Ok(Flow::Stop)
        } else {
            let index = p.index;
            p.index += 1;
            let meter = p.meter.clone();
            let step = Step {
                index,
                elapsed,
                meter: &meter,
            };
            (p.step)(&step, window, cx)
        }
    };
    match flow {
        Ok(Flow::Continue) => schedule(pacer, window),
        Ok(Flow::Stop) => finish(&pacer, Ok(())),
        Err(err) => finish(&pacer, Err(err)),
    }
}

fn finish(pacer: &Rc<RefCell<Pacer>>, result: Result<()>) {
    if let Some(done) = pacer.borrow_mut().done.take() {
        let _ = done.send(result);
    }
}
