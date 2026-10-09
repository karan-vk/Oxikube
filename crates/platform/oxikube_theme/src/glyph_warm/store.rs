//! What the warmer knows: the glyphs GPUI drew, the jobs queued from them, the bitmaps prepared.

use super::plan::{DilationPlan, LEVELS};
use super::warmer::WarmStats;
use gpui::{Bounds, DevicePixels, RenderGlyphParams, Size};
use std::collections::{HashMap, VecDeque};

/// What is known of one glyph (its parameters at dilation 0): bit masks of levels.
#[derive(Clone, Copy, Default)]
struct Glyph {
    /// Levels GPUI drew it at (rasterised in a frame or served prepared): in the atlas.
    drawn: u8,
    /// `drawn` plus the levels queued or prepared: never queued again.
    known: u8,
}

pub(super) struct Prepared {
    pub(super) bounds: Bounds<DevicePixels>,
    pub(super) size: Size<DevicePixels>,
    pub(super) bytes: Vec<u8>,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) plan: DilationPlan,
    /// The plan changed: every known glyph is to be checked against it.
    pub(super) rescan: bool,
    glyphs: HashMap<RenderGlyphParams, Glyph>,
    pub(super) queue: VecDeque<RenderGlyphParams>,
    pub(super) prepared: HashMap<RenderGlyphParams, Prepared>,
    pub(super) stats: WarmStats,
    pub(super) worker_started: bool,
}

impl State {
    /// How many distinct glyphs GPUI has drawn.
    pub(super) fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }

    /// Ends a rescan: queues, for every glyph drawn so far, the levels the new plan wants.
    pub(super) fn queue_plan_levels(&mut self) {
        self.rescan = false;
        let Self {
            plan,
            glyphs,
            queue,
            ..
        } = self;
        for (key, glyph) in glyphs.iter_mut() {
            queue_missing(plan, key, glyph, queue);
        }
    }
}

/// `params` at dilation `level`.
pub(super) fn at_level(params: &RenderGlyphParams, level: u8) -> RenderGlyphParams {
    RenderGlyphParams {
        dilation: level,
        ..params.clone()
    }
}

/// Whether GPUI drew `params` itself (at its level).
pub(super) fn is_drawn(state: &State, params: &RenderGlyphParams) -> bool {
    let key = at_level(params, 0);
    state
        .glyphs
        .get(&key)
        .is_some_and(|glyph| glyph.drawn & (1 << params.dilation) != 0)
}

/// Marks `params`'s level drawn and queues the plan's levels it is not known at; whether it
/// queued any.
pub(super) fn record_drawn(state: &mut State, params: &RenderGlyphParams) -> bool {
    if params.dilation >= LEVELS {
        return false;
    }
    let key = at_level(params, 0);
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
        queue.push_back(at_level(key, level));
    }
    true
}
