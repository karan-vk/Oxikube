//! The frame hook: a root view that times every frame it is part of.

use super::recorder::Recorder;
use gpui::{AnyView, Context, IntoElement, Render, Window};
use std::sync::Arc;
use std::time::Instant;

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
/// callback and one lock-free ring push.
pub struct PerfRoot {
    inner: AnyView,
    recorder: Arc<Recorder>,
}

impl PerfRoot {
    /// Wraps `inner` (usually `cx.new(..).into()`).
    pub fn new(inner: impl Into<AnyView>, recorder: Arc<Recorder>) -> Self {
        Self {
            inner: inner.into(),
            recorder,
        }
    }

    /// The wrapped root view.
    pub fn inner(&self) -> &AnyView {
        &self.inner
    }
}

impl Render for PerfRoot {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let start = Instant::now();
        let recorder = self.recorder.clone();
        cx.defer(move |_| recorder.record_frame(start.elapsed()));
        self.inner.clone()
    }
}
