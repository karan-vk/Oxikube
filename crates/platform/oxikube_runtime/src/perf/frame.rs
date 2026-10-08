//! The frame hook: a root view that times every frame it is part of.

use super::recorder::{FrameNotifies, Recorder};
use gpui::{AnyView, Context, IntoElement, Render, Window};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One frame as the hook saw it, handed to a [`FrameTap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSample {
    /// When GPUI asked the root view to render: the start of `Window::draw`.
    pub start: Instant,
    /// From `start` to the end of the update that drew (and presented) the frame.
    pub duration: Duration,
    /// The coalesced notifies the frame absorbed.
    pub notifies: FrameNotifies,
}

/// Called on the UI thread at the end of every frame the hook records (after `present`), with
/// that frame. The windowed scenarios install one to attribute frames to their scripted phases
/// (`perf::windowed`).
pub type FrameTap = Rc<dyn Fn(FrameSample)>;

/// Wraps a window's real root view and records one frame per draw into a [`Recorder`].
///
/// GPUI renders the root view at the start of every `Window::draw` (root views are never served
/// from the view cache), so `render` is the frame start. It schedules an `App::defer` callback,
/// which runs when GPUI flushes effects at the end of the update that is drawing, i.e. after
/// `draw` and (in the windowed app) `present` returned; that is the frame end. See the module docs
/// of [`crate::perf`] for exactly what is and is not covered.
///
/// Only install it when `--perf` is on: with `--perf` off the window holds the real root directly
/// and pays nothing. When on, the cost per frame is one `Instant::now()` pair, one boxed deferred
/// callback and one lock-free ring push (plus the [`FrameTap`] call when one is set).
pub struct PerfRoot {
    inner: AnyView,
    recorder: Arc<Recorder>,
    tap: Option<FrameTap>,
}

impl PerfRoot {
    /// Wraps `inner` (usually `cx.new(..).into()`).
    pub fn new(inner: impl Into<AnyView>, recorder: Arc<Recorder>) -> Self {
        Self {
            inner: inner.into(),
            recorder,
            tap: None,
        }
    }

    /// The wrapped root view.
    pub fn inner(&self) -> &AnyView {
        &self.inner
    }

    /// Sets (or with `None` removes) the callback that sees every recorded frame.
    pub fn set_tap(&mut self, tap: Option<FrameTap>) {
        self.tap = tap;
    }
}

impl Render for PerfRoot {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let start = Instant::now();
        let recorder = self.recorder.clone();
        let tap = self.tap.clone();
        cx.defer(move |_| {
            let duration = start.elapsed();
            let notifies = recorder.record_frame(duration);
            if let Some(tap) = tap {
                tap(FrameSample {
                    start,
                    duration,
                    notifies,
                });
            }
        });
        self.inner.clone()
    }
}
