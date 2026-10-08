//! The frame hook: a root view that times every frame it is part of.

use super::recorder::{FrameNotifies, Recorder};
use gpui::{
    AnyElement, AnyView, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Window, canvas, deferred, div,
};
use std::cell::Cell;
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
    /// From `start` to the end of the content's paint: layout, prepaint and paint of the tree and
    /// of its deferred overlays (popovers, menus, dropdowns), before the scene is finished and
    /// handed to the renderer, whose `present` waits for a free drawable (on macOS about until the
    /// next refresh while the window draws every refresh). GPUI paints the window's one tooltip,
    /// in-window prompt or drag preview after every deferred draw, so that element's paint (its
    /// layout and prepaint are inside) is outside `drawn` and inside `duration`. Only measured
    /// while a [`FrameTap`] is set.
    pub drawn: Option<Duration>,
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
        let painted = Rc::new(Cell::new(None));
        let drawn = painted.clone();
        cx.defer(move |_| {
            let duration = start.elapsed();
            let notifies = recorder.record_frame(duration);
            if let Some(tap) = tap {
                tap(FrameSample {
                    start,
                    duration,
                    drawn: drawn
                        .get()
                        .map(|end: Instant| end.saturating_duration_since(start)),
                    notifies,
                });
            }
        });
        if self.tap.is_none() {
            return self.inner.clone().into_any_element();
        }
        paint_probe(self.inner.clone(), painted)
    }
}

/// The deferred-draw priority of the probe: above any overlay's, so it paints last.
const PROBE_PRIORITY: usize = usize::MAX;

/// `inner`, followed by an empty element that notes when the content's paint ended. Only with a
/// tap: without one the root is the content alone.
///
/// In gpui-pre 0.3.7 `Window::draw_roots` prepaints and paints the root tree, then the deferred
/// draws (`deferred(..)`: popovers, menus, dropdowns) in priority order, then the window's tooltip,
/// prompt or drag preview. The probe is a deferred draw at [`PROBE_PRIORITY`], so its paint runs
/// after every deferred overlay's prepaint and paint.
fn paint_probe(inner: AnyView, painted: Rc<Cell<Option<Instant>>>) -> AnyElement {
    div()
        .id("perf-root")
        .size_full()
        .child(inner)
        .child(
            deferred(
                canvas(
                    |_, _, _| {},
                    move |_, _, _, _| painted.set(Some(Instant::now())),
                )
                .absolute()
                .size_0(),
            )
            .with_priority(PROBE_PRIORITY),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, TestAppContext};
    use std::time::Duration;

    /// Content with a deferred overlay that notes when its paint ended (after spinning, so the
    /// order cannot hide inside the clock's resolution).
    struct WithOverlay(Rc<Cell<Option<Instant>>>);

    impl Render for WithOverlay {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let painted = self.0.clone();
            div().size_full().child(deferred(
                canvas(
                    |_, _, _| {},
                    move |_, _, _, _| {
                        let until = Instant::now() + Duration::from_millis(2);
                        while Instant::now() < until {}
                        painted.set(Some(Instant::now()));
                    },
                )
                .size_0(),
            ))
        }
    }

    /// `drawn` ends after the deferred overlays' paint, not at the end of the root tree's.
    #[gpui::test]
    fn drawn_covers_the_paint_of_deferred_overlays(cx: &mut TestAppContext) {
        let overlay = Rc::new(Cell::new(None));
        let samples = Rc::new(std::cell::RefCell::new(Vec::new()));
        let tap: FrameTap = {
            let samples = samples.clone();
            Rc::new(move |frame| samples.borrow_mut().push(frame))
        };
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                let inner = cx.new(|_| WithOverlay(overlay.clone()));
                cx.new(|_| {
                    let mut root = PerfRoot::new(inner, Arc::new(Recorder::new()));
                    root.set_tap(Some(tap));
                    root
                })
            })
            .unwrap()
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        })
        .unwrap();
        cx.run_until_parked();
        let frame = *samples.borrow().last().expect("a frame was recorded");
        let overlay_end = overlay.get().expect("the overlay was painted");
        let drawn_end = frame.start + frame.drawn.expect("the probe was painted");
        assert!(
            drawn_end >= overlay_end,
            "drawn ends {:?} before the overlay's paint",
            overlay_end - drawn_end
        );
    }
}
