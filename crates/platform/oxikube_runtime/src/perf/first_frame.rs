//! The first-frame marker: the moment start-up ends (E05-S13, docs/PERFORMANCE.md "Startup").

use gpui::{AnyView, App, Context, IntoElement, Render, Window};

/// Wraps a window's content and runs a callback once, at the end of the update that drew the
/// window's first frame: after `draw` and (in the windowed app) `present` returned, the same frame
/// end [`PerfRoot`](super::PerfRoot) uses. From then on the window is interactive: its focus tree
/// and hit boxes exist, so key bindings, actions and clicks dispatch.
///
/// Cheap enough to leave on in every build: after the first frame `render` only clones the inner
/// view handle.
pub struct FirstFrameProbe {
    inner: AnyView,
    on_first_frame: Option<Box<dyn FnOnce(&mut App)>>,
}

impl FirstFrameProbe {
    /// Wraps `inner`; `on_first_frame` runs once, when the first frame that contains it is done.
    pub fn new(inner: impl Into<AnyView>, on_first_frame: impl FnOnce(&mut App) + 'static) -> Self {
        Self {
            inner: inner.into(),
            on_first_frame: Some(Box::new(on_first_frame)),
        }
    }

    /// The wrapped view.
    pub fn inner(&self) -> &AnyView {
        &self.inner
    }

    /// Whether the first frame has been drawn (the callback is scheduled or ran).
    pub fn fired(&self) -> bool {
        self.on_first_frame.is_none()
    }
}

impl Render for FirstFrameProbe {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(callback) = self.on_first_frame.take() {
            // Deferred effects run when the drawing update flushes, i.e. after draw and present.
            cx.defer(callback);
        }
        self.inner.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, TestAppContext, div};
    use std::cell::Cell;
    use std::rc::Rc;

    struct Content;

    impl Render for Content {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui::test]
    fn fires_once_after_the_first_frame(cx: &mut TestAppContext) {
        let fired = Rc::new(Cell::new(0));
        let counter = fired.clone();
        let window = cx.add_window(move |_, cx| {
            let content = cx.new(|_| Content);
            FirstFrameProbe::new(content, move |_| counter.set(counter.get() + 1))
        });
        cx.run_until_parked();
        assert_eq!(fired.get(), 1);
        window
            .update(cx, |probe, window, cx| {
                assert!(probe.fired());
                window.refresh();
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(fired.get(), 1, "only the first frame");
    }
}
