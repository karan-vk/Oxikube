//! The view's session: opening it, reopening it when what is read changes, and the pump that
//! applies its deltas.
//!
//! The pump is one foreground task the view owns (replaced, never cleared from inside, when the
//! session is). Each wake takes the delta that is ready (the session computes it at poll time from
//! the view's cursor, so a burst of batches is one delta), applies it in one update and redraws
//! through `notify_coalesced`: at most one redraw per frame however fast the pod writes.

use futures::StreamExt as _;
use gpui::Context;
use oxikube_app::logs::{LogDelta, LogFailure, LogState, LogTarget};
use oxikube_domain::ErrorKind;
use oxikube_runtime::notify_coalesced;

use super::LogView;
use super::window::LineWindow;

impl LogView {
    /// Opens the session for the current options, dropping the previous one first (which
    /// cancels its read). The rows start over: a new session numbers its lines from 0.
    ///
    /// While the view waits for the pod to name its default container nothing opens: the
    /// pod's arrival opens the stream with the options as they are then, so a stream is never
    /// read from a container other than the one the tab names.
    pub(crate) fn open_stream(&mut self, cx: &mut Context<Self>) {
        if self.awaiting_pod {
            return;
        }
        self.pump = None;
        self.session = None;
        self.started = true;
        self.follow.restart();
        let port = self
            .deps
            .sessions
            .get(&self.target.cluster)
            .and_then(|session| session.logs());
        let (Some(port), Some(namespace)) = (port, self.target.namespace.as_deref()) else {
            self.reset_rows(LineWindow::with_state(not_connected()));
            notify_coalesced(cx);
            return;
        };
        let target = LogTarget {
            namespace: namespace.to_owned(),
            pod: self.target.name.to_string(),
            container: self.options.container.clone(),
        };
        let session = self.deps.service.open_in(
            &self.target.cluster,
            port,
            target,
            self.options.log_options(),
        );
        let mut deltas = session.deltas();
        self.session = Some(session);
        self.reset_rows(LineWindow::new());
        self.pump = Some(cx.spawn(async move |this, cx| {
            // Each poll computes one delta from the view's cursor: everything committed since
            // the last one, however many batches that was.
            while let Some(delta) = deltas.next().await {
                if this
                    .update(cx, |view, cx| view.apply_delta(&delta, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
        notify_coalesced(cx);
    }

    /// Applies one delta: the rows, the renderers, autoscroll.
    pub(crate) fn apply_delta(&mut self, delta: &LogDelta, cx: &mut Context<Self>) {
        let change = self.window.apply(delta);
        self.rows_changed(change);
        self.follow_tail();
        notify_coalesced(cx);
    }
}

/// The state row of a view whose cluster has no connection (or whose target names no pod).
fn not_connected() -> LogState {
    LogState::Failed(LogFailure {
        kind: ErrorKind::Network,
        message: "the cluster is not connected".to_owned(),
        retryable: true,
    })
}
