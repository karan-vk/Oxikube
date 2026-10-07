//! After the stream stopped (E08-S07): following the pod that replaced a gone one
//! (`logs::FollowReplacement`), reconnecting a stream that failed or closed (`logs::Reconnect`),
//! and the strip under the toolbar that offers them.
//!
//! Following a replacement reads the gone pod's controller off the UI thread
//! (`oxikube_app::logs::find_replacement` on `spawn_kube`) and switches the tab to the pod that
//! took over: same container and range, the tab renamed. A reconnect keeps the lines the session
//! holds and continues after them (`LogSession::reconnect`); a multi-pod view reopens its streams.

use gpui::Context;
use oxikube_app::logs::{EndReason, LogState, find_replacement};
use oxikube_domain::command::Command;
use oxikube_domain::redact::redact;
use oxikube_domain::{ErrorKind, OxiResult};
use oxikube_runtime::{KubeTaskError, spawn_kube};
use oxikube_workspace::{ItemEvent, Toast};

use super::LogView;

/// What the view offers once its stream stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// The pod was replaced: switch to the pod that took over (`logs::FollowReplacement`).
    FollowReplacement,
    /// Open the stream again (`logs::Reconnect`): it failed, or closed for a reason the view
    /// could not tell.
    Reconnect,
    /// The pod or container is gone (the read failed with `NotFound`): reading it again cannot
    /// help, so the strip offers to close the tab instead.
    Close,
}

impl LogView {
    /// What the strip under the toolbar offers now, if anything.
    pub fn recovery(&self) -> Option<Recovery> {
        match self.window.state() {
            LogState::Ended(EndReason::PodReplaced) if self.aggregate.is_none() => {
                Some(Recovery::FollowReplacement)
            }
            LogState::Failed(failure) if failure.kind == ErrorKind::NotFound => {
                Some(Recovery::Close)
            }
            LogState::Ended(EndReason::StreamClosed) | LogState::Failed(_) => {
                Some(Recovery::Reconnect)
            }
            _ => None,
        }
    }

    /// Opens the stream again after it ended or failed: a pod's session keeps its lines and
    /// continues after them; a multi-pod view, or a view without a session, opens anew. Nothing
    /// happens while the stream still reads.
    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        let state = self.window.state();
        if !state.is_terminal() || matches!(state, LogState::Ended(EndReason::Cancelled)) {
            return;
        }
        // Gone is gone: the strip offers Close, and the `r` key does not reopen a read that
        // cannot succeed.
        if self.recovery() == Some(Recovery::Close) {
            return;
        }
        self.error_details_open = false;
        let resumed = match (&self.aggregate, self.session.as_mut()) {
            (None, Some(session)) => session.reconnect(),
            _ => false,
        };
        if resumed {
            self.pump_session(cx);
        } else {
            self.open_stream(cx);
        }
    }

    /// Looks for the pod that replaced the view's pod (off the UI thread) and switches to it; a
    /// toast says when there is none yet. Only once the pod is gone: a running pod has no
    /// replacement.
    pub fn follow_replacement(&mut self, cx: &mut Context<Self>) {
        if self.aggregate.is_some() {
            return;
        }
        if !matches!(
            self.window.state(),
            LogState::Ended(EndReason::PodReplaced | EndReason::PodDeleted)
        ) {
            let toast = Toast::info("The pod is still there: there is no replacement to follow.");
            self.toast(toast, cx);
            return;
        }
        let gone = self.session.as_ref().and_then(|s| s.pod_identity());
        let resources = self
            .deps
            .sessions
            .get(&self.target.cluster)
            .and_then(|s| s.resources());
        let (Some(gone), Some(resources)) = (gone, resources) else {
            let toast = Toast::info("The pod's owner is not known: there is no replacement.");
            self.toast(toast, cx);
            return;
        };
        let lookup = spawn_kube(cx, async move {
            find_replacement(resources.as_ref(), &gone).await
        });
        self.replacement_task = Some(cx.spawn(async move |this, cx| {
            let found = lookup.await;
            this.update(cx, |view, cx| view.replacement_found(found, cx))
                .ok();
        }));
    }

    fn replacement_found(
        &mut self,
        found: Result<OxiResult<Option<String>>, KubeTaskError>,
        cx: &mut Context<Self>,
    ) {
        match found {
            Ok(Ok(Some(pod))) => self.switch_pod(&pod, cx),
            Ok(Ok(None)) => {
                let gone = self.target.name.clone();
                let toast = Toast::info(format!(
                    "No pod has replaced {gone} yet: try again in a moment."
                ));
                self.toast(toast, cx);
            }
            Ok(Err(error)) => {
                let message = redact(error.message()).into_owned();
                let toast =
                    Toast::warning(format!("Could not look for the replacement: {message}"));
                self.toast(toast, cx);
            }
            Err(_) => {}
        }
    }

    /// Shows `pod` (of the same namespace) in this tab instead: the same container and range,
    /// the tab renamed, the stream opened anew.
    fn switch_pod(&mut self, pod: &str, cx: &mut Context<Self>) {
        let gone = std::mem::replace(&mut self.target.name, pod.into());
        self.pump = None;
        self.session = None;
        self.started = false;
        self.containers.clear();
        cx.emit(ItemEvent::UpdateTab);
        if self.options.container.is_some() {
            self.open_stream(cx);
        }
        self.load_pod(cx);
        let toast = Toast::success(format!("Following pod {pod}, which replaced {gone}."));
        self.toast(toast, cx);
        cx.notify();
    }

    /// Asks to follow the replacement pod (`logs::FollowReplacement`).
    pub fn request_follow_replacement(&mut self, cx: &mut Context<Self>) {
        let target = self.target.clone();
        self.send(Command::LogsFollowReplacement { target }, cx);
    }

    /// Asks to reconnect (`logs::Reconnect`).
    pub fn request_reconnect(&mut self, cx: &mut Context<Self>) {
        let target = self.target.clone();
        self.send(Command::LogsReconnect { target }, cx);
    }
}
