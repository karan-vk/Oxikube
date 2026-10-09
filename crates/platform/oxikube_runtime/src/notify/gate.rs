//! [`RenderGate`]: a streaming view's coalesced notify, skipped until the view has rendered.

use gpui::Context;

/// A streaming view's coalesced notify that is skipped while the view has not rendered since the
/// last one (E05-P599).
///
/// A view notified again before it renders gains nothing from it. If the view is on screen, the
/// first notify already dirtied its window, which draws the view with everything applied since.
/// If it is not on screen (a background cluster tab), GPUI renders it from its current state when
/// it is shown. Without the gate such a hidden view is notified on every frame its stream changes,
/// although its window draws none of them: the notifies cost observer runs and frame requests, and
/// several of them land between two drawn frames.
///
/// Use: call [`notify`](Self::notify) where the view would call
/// [`notify_coalesced`](super::notify_coalesced), and [`rendered`](Self::rendered) in its `render`.
/// Observers of the view hear one notify per render, which is what a redraw-driven observer needs.
/// Not for entities that are never rendered (models): their observers would stop hearing changes.
#[derive(Debug, Default)]
pub struct RenderGate {
    /// A notify was sent and the view has not rendered since.
    awaiting_render: bool,
}

impl RenderGate {
    /// Coalesces a notify for the view, unless one is already waiting for its render.
    pub fn notify<T: 'static>(&mut self, cx: &mut Context<T>) {
        if !self.awaiting_render {
            self.awaiting_render = true;
            super::notify_coalesced(cx);
        }
    }

    /// The view rendered: the next change notifies again. Call it from the view's `render`.
    pub fn rendered(&mut self) {
        self.awaiting_render = false;
    }

    /// Whether a notify was sent and the view has not rendered since.
    pub fn awaiting_render(&self) -> bool {
        self.awaiting_render
    }
}
