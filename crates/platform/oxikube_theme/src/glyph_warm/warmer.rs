//! [`GlyphWarmer`]: the store of glyphs rasterised ahead of a theme switch, and the worker thread
//! that fills it.
//!
//! The platform text system stays the only rasteriser: the worker calls the same
//! `glyph_raster_bounds` and `rasterize_glyph` GPUI would call in the frame, with the same
//! parameters, so a prepared glyph is byte for byte the one GPUI would have drawn.

use super::plan::{DilationPlan, LEVELS};
use gpui::{Bounds, DevicePixels, PlatformTextSystem, RenderGlyphParams, Size};
use parking_lot::{Condvar, Mutex};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

/// How long the text system must have been left alone before the worker rasterises: the platform
/// text system shapes text under a write lock that a rasterisation in flight holds off, so the
/// worker runs between frames, never alongside one's text work (at most one glyph, tens of
/// microseconds, can overlap the start of the next frame).
pub const QUIET: Duration = Duration::from_millis(2);

/// The most bytes of prepared glyphs kept waiting for a switch (8 MiB; a glyph of the UI font at
/// scale 2 is well under a kilobyte). Past it the worker stops preparing; the frame that needs a
/// glyph it skipped rasterises it as before.
pub const PREPARED_BUDGET: usize = 8 << 20;

/// Counters of what the warmer did, for tests and the perf report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WarmStats {
    /// Glyphs rasterised ahead by the worker.
    pub prepared: u64,
    /// Rasterisations GPUI asked for that a prepared glyph answered (no rasterisation in frame).
    pub served: u64,
    /// Rasterisations GPUI asked for that the platform did in the frame (no prepared glyph).
    pub rasterised: u64,
    /// Jobs dropped: an empty glyph, a platform error, or the [`PREPARED_BUDGET`] reached.
    pub skipped: u64,
    /// Jobs waiting for the worker.
    pub pending: usize,
    /// Bytes of prepared glyphs not yet asked for.
    pub prepared_bytes: usize,
}

/// See the [module docs](super). Cheap to clone (a handle).
#[derive(Clone)]
pub struct GlyphWarmer {
    pub(super) shared: Arc<Shared>,
}

/// The state behind a [`GlyphWarmer`]; also the decorating text system GPUI holds
/// (`text_system.rs`).
pub(super) struct Shared {
    /// Itself, for the worker thread it starts.
    this: Weak<Shared>,
    pub(super) inner: Arc<dyn PlatformTextSystem>,
    state: Mutex<State>,
    wake: Condvar,
    epoch: Instant,
    /// Nanoseconds after `epoch` of the last call GPUI made into the text system.
    last_activity: AtomicU64,
    background: bool,
}

/// What is known of one glyph (its parameters at dilation 0): bit masks of levels.
#[derive(Clone, Copy, Default)]
struct Glyph {
    /// Levels GPUI drew it at (rasterised in a frame or served prepared): in the atlas.
    drawn: u8,
    /// `drawn` plus the levels queued or prepared: never queued again.
    known: u8,
}

struct Prepared {
    bounds: Bounds<DevicePixels>,
    size: Size<DevicePixels>,
    bytes: Vec<u8>,
}

#[derive(Default)]
struct State {
    plan: DilationPlan,
    /// The plan changed: every known glyph is to be checked against it.
    rescan: bool,
    glyphs: HashMap<RenderGlyphParams, Glyph>,
    queue: VecDeque<RenderGlyphParams>,
    prepared: HashMap<RenderGlyphParams, Prepared>,
    stats: WarmStats,
    worker_started: bool,
}

impl GlyphWarmer {
    /// A warmer over the platform's text system, with a worker thread (started with the first
    /// job) that rasterises ahead.
    pub fn new(inner: Arc<dyn PlatformTextSystem>) -> Self {
        Self::build(inner, true)
    }

    /// A warmer without a worker thread: jobs wait for [`warm_pending`](Self::warm_pending).
    /// For tests, which must not start threads.
    pub fn manual(inner: Arc<dyn PlatformTextSystem>) -> Self {
        Self::build(inner, false)
    }

    fn build(inner: Arc<dyn PlatformTextSystem>, background: bool) -> Self {
        Self {
            shared: Arc::new_cyclic(|this| Shared {
                this: this.clone(),
                inner,
                state: Mutex::new(State::default()),
                wake: Condvar::new(),
                epoch: Instant::now(),
                last_activity: AtomicU64::new(0),
                background,
            }),
        }
    }

    /// The text system to hand GPUI in place of the platform's (see `GlyphWarmPlatform`).
    pub fn text_system(&self) -> Arc<dyn PlatformTextSystem> {
        self.shared.clone()
    }

    /// The level the platform draws text of `color` at.
    pub fn dilation_for_color(&self, color: gpui::Hsla) -> u8 {
        self.shared.inner.glyph_dilation_for_color(color)
    }

    /// The plan in effect.
    pub fn plan(&self) -> DilationPlan {
        self.shared.state.lock().plan
    }

    /// Sets the levels to warm every drawn glyph at; a changed plan re-checks every glyph drawn so
    /// far (on the worker, not the caller's thread).
    pub fn set_plan(&self, plan: DilationPlan) {
        let mut state = self.shared.state.lock();
        if state.plan == plan {
            return;
        }
        tracing::debug!(
            ?plan,
            glyphs = state.glyphs.len(),
            "glyph warm-up: new plan"
        );
        state.plan = plan;
        state.rescan = true;
        self.shared.wake_worker(&mut state);
    }

    /// Runs every waiting job on the calling thread, ignoring [`QUIET`]; returns how many ran.
    /// What the worker does, for a [`manual`](Self::manual) warmer.
    pub fn warm_pending(&self) -> usize {
        let mut ran = 0;
        loop {
            self.shared.rescan_if_needed();
            if !self.shared.warm_one() {
                return ran;
            }
            ran += 1;
        }
    }

    /// The counters so far.
    pub fn stats(&self) -> WarmStats {
        let state = self.shared.state.lock();
        WarmStats {
            pending: state.queue.len(),
            ..state.stats
        }
    }
}

impl Shared {
    /// Notes that GPUI is using the text system now.
    pub(super) fn touch(&self) {
        let now = self.epoch.elapsed().as_nanos() as u64;
        self.last_activity.store(now, Ordering::Relaxed);
    }

    fn since_activity(&self) -> Duration {
        let now = self.epoch.elapsed().as_nanos() as u64;
        Duration::from_nanos(now.saturating_sub(self.last_activity.load(Ordering::Relaxed)))
    }

    /// The bounds of a prepared glyph, so GPUI's bounds lookup for it costs no platform call.
    pub(super) fn prepared_bounds(
        &self,
        params: &RenderGlyphParams,
    ) -> Option<Bounds<DevicePixels>> {
        if params.is_emoji {
            return None;
        }
        self.state.lock().prepared.get(params).map(|p| p.bounds)
    }

    /// Hands over the prepared bitmap of `params` (rasterised at `bounds`), or `None`.
    pub(super) fn take_prepared(
        &self,
        params: &RenderGlyphParams,
        bounds: Bounds<DevicePixels>,
    ) -> Option<(Size<DevicePixels>, Vec<u8>)> {
        if params.is_emoji {
            return None;
        }
        let mut state = self.state.lock();
        let prepared = state.prepared.remove(params)?;
        state.stats.prepared_bytes -= prepared.bytes.len();
        if prepared.bounds != bounds {
            return None;
        }
        state.stats.served += 1;
        if record_drawn(&mut state, params) {
            self.wake_worker(&mut state);
        }
        Some((prepared.size, prepared.bytes))
    }

    /// Records a glyph GPUI rasterised in a frame, and queues it at the plan's levels.
    pub(super) fn rasterised(&self, params: &RenderGlyphParams) {
        if params.is_emoji {
            return;
        }
        let mut state = self.state.lock();
        state.stats.rasterised += 1;
        if record_drawn(&mut state, params) {
            self.wake_worker(&mut state);
        }
    }

    fn wake_worker(&self, state: &mut State) {
        if !self.background {
            return;
        }
        if !state.worker_started {
            let Some(shared) = self.this.upgrade() else {
                return;
            };
            state.worker_started = true;
            let spawned = std::thread::Builder::new()
                .name("oxikube-glyph-warm".into())
                .spawn(move || shared.run_worker());
            if let Err(err) = spawned {
                tracing::warn!(%err, "glyph warm-up is off: cannot start its thread");
                return;
            }
        }
        self.wake.notify_one();
    }

    fn run_worker(self: Arc<Self>) {
        tracing::debug!("glyph warm-up: worker started");
        loop {
            {
                let mut state = self.state.lock();
                while state.queue.is_empty() && !state.rescan {
                    self.wake.wait(&mut state);
                }
            }
            self.rescan_if_needed();
            self.wait_for_quiet();
            self.warm_one();
        }
    }

    fn wait_for_quiet(&self) {
        loop {
            let since = self.since_activity();
            if since >= QUIET {
                return;
            }
            std::thread::sleep(QUIET - since);
        }
    }

    fn rescan_if_needed(&self) {
        let mut state = self.state.lock();
        if !state.rescan {
            return;
        }
        state.rescan = false;
        let State {
            plan,
            glyphs,
            queue,
            ..
        } = &mut *state;
        for (key, glyph) in glyphs.iter_mut() {
            queue_missing(plan, key, glyph, queue);
        }
    }

    /// Rasterises the next job, if any, outside the lock. A job GPUI has meanwhile drawn itself
    /// (it is in the atlas, nobody would ask for it) is dropped.
    fn warm_one(&self) -> bool {
        let job = {
            let mut state = self.state.lock();
            let Some(job) = state.queue.pop_front() else {
                return false;
            };
            if is_drawn(&state, &job) {
                state.stats.skipped += 1;
                return true;
            }
            job
        };
        let raster = match self.inner.glyph_raster_bounds(&job) {
            Ok(bounds) if !bounds.is_empty() => {
                self.inner
                    .rasterize_glyph(&job, bounds)
                    .ok()
                    .map(|(size, bytes)| Prepared {
                        bounds,
                        size,
                        bytes,
                    })
            }
            _ => None,
        };
        let mut state = self.state.lock();
        match raster {
            Some(prepared)
                if !is_drawn(&state, &job)
                    && state.stats.prepared_bytes + prepared.bytes.len() <= PREPARED_BUDGET =>
            {
                state.stats.prepared += 1;
                state.stats.prepared_bytes += prepared.bytes.len();
                state.prepared.insert(job, prepared);
            }
            _ => state.stats.skipped += 1,
        }
        true
    }
}

/// Whether GPUI drew `params` itself (at its level).
fn is_drawn(state: &State, params: &RenderGlyphParams) -> bool {
    let key = RenderGlyphParams {
        dilation: 0,
        ..params.clone()
    };
    state
        .glyphs
        .get(&key)
        .is_some_and(|glyph| glyph.drawn & (1 << params.dilation) != 0)
}

/// Marks `params`'s level drawn and queues the plan's levels it is not known at; whether it
/// queued any.
fn record_drawn(state: &mut State, params: &RenderGlyphParams) -> bool {
    if params.dilation >= LEVELS {
        return false;
    }
    let key = RenderGlyphParams {
        dilation: 0,
        ..params.clone()
    };
    let State {
        plan,
        glyphs,
        queue,
        ..
    } = state;
    let glyph = glyphs.entry(key.clone()).or_default();
    let bit = 1 << params.dilation;
    glyph.drawn |= bit;
    glyph.known |= bit;
    queue_missing(plan, &key, glyph, queue)
}

/// Queues `key` at the levels `plan` wants for what it was drawn at and it is not known at.
fn queue_missing(
    plan: &DilationPlan,
    key: &RenderGlyphParams,
    glyph: &mut Glyph,
    queue: &mut VecDeque<RenderGlyphParams>,
) -> bool {
    let want = plan.targets_of(glyph.drawn) & !glyph.known;
    if want == 0 {
        return false;
    }
    glyph.known |= want;
    for level in (0..LEVELS).filter(|level| want & (1 << level) != 0) {
        queue.push_back(RenderGlyphParams {
            dilation: level,
            ..key.clone()
        });
    }
    true
}
